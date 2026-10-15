//! The host's SQLite file: paired devices and per-device usage counts (`P2-T9`, `P4-T4`).
//!
//! **Prompt and reply text is never stored** (spec §6.8, §7). There is no column that could
//! hold it, and `tests/no_prompt_text_stored.rs` greps the raw database files after a real chat
//! to keep it that way.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
#[error("host database: {0}")]
pub struct StoreError(#[from] rusqlite::Error);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Certificate fingerprint.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub requests: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub gpu_ms: u64,
}

pub struct Store {
    db: Mutex<Connection>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS devices (
    id           TEXT PRIMARY KEY,   -- certificate fingerprint
    name         TEXT NOT NULL,
    token_sha256 TEXT NOT NULL,
    paired_at    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS usage (
    device_id  TEXT NOT NULL,
    day        TEXT NOT NULL,        -- YYYY-MM-DD, UTC
    requests   INTEGER NOT NULL DEFAULT 0,
    tokens_in  INTEGER NOT NULL DEFAULT 0,
    tokens_out INTEGER NOT NULL DEFAULT 0,
    gpu_ms     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (device_id, day)
);
";

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { db: Mutex::new(conn) })
    }

    /// Records a newly paired device and the hash of its token. Re-pairing replaces both.
    pub fn pair(&self, id: &str, name: &str, token: &str, now_unix: i64) -> Result<(), StoreError> {
        self.db.lock().unwrap().execute(
            "INSERT INTO devices (id, name, token_sha256, paired_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, token_sha256 = excluded.token_sha256,
                                           paired_at = excluded.paired_at",
            params![id, name, token_hash(token), now_unix],
        )?;
        Ok(())
    }

    /// Removes a device. Its usage history is kept (it's counts only).
    pub fn unpair(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self.db.lock().unwrap().execute("DELETE FROM devices WHERE id = ?1", params![id])? > 0)
    }

    pub fn devices(&self) -> Result<Vec<Device>, StoreError> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare("SELECT id, name FROM devices ORDER BY paired_at, id")?;
        let rows = st.query_map([], |r| Ok(Device { id: r.get(0)?, name: r.get(1)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The paired device with this fingerprint, if `token` is its token.
    pub fn authorize(&self, id: &str, token: &str) -> Result<Option<Device>, StoreError> {
        let db = self.db.lock().unwrap();
        let row: Option<(String, String)> = db
            .query_row("SELECT name, token_sha256 FROM devices WHERE id = ?1", params![id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        Ok(row.and_then(|(name, stored)| {
            constant_time_eq(stored.as_bytes(), token_hash(token).as_bytes()).then(|| Device { id: id.into(), name })
        }))
    }

    pub fn add_usage(&self, device: &str, day: &str, u: Usage) -> Result<(), StoreError> {
        self.db.lock().unwrap().execute(
            "INSERT INTO usage (device_id, day, requests, tokens_in, tokens_out, gpu_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(device_id, day) DO UPDATE SET
               requests = requests + excluded.requests, tokens_in = tokens_in + excluded.tokens_in,
               tokens_out = tokens_out + excluded.tokens_out, gpu_ms = gpu_ms + excluded.gpu_ms",
            params![device, day, u.requests, u.tokens_in, u.tokens_out, u.gpu_ms],
        )?;
        Ok(())
    }

    /// Totals per device for days `>= since_day` (YYYY-MM-DD).
    pub fn usage_since(&self, since_day: &str) -> Result<Vec<(String, Usage)>, StoreError> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(
            "SELECT device_id, SUM(requests), SUM(tokens_in), SUM(tokens_out), SUM(gpu_ms)
             FROM usage WHERE day >= ?1 GROUP BY device_id",
        )?;
        let rows = st.query_map(params![since_day], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Usage {
                    requests: r.get::<_, i64>(1)? as u64,
                    tokens_in: r.get::<_, i64>(2)? as u64,
                    tokens_out: r.get::<_, i64>(3)? as u64,
                    gpu_ms: r.get::<_, i64>(4)? as u64,
                },
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_right_token_from_the_right_device_is_accepted() {
        let s = Store::in_memory().unwrap();
        s.pair("fp-a", "Tunde's PC", "secret-a", 1).unwrap();
        assert_eq!(s.authorize("fp-a", "secret-a").unwrap().unwrap().name, "Tunde's PC");
        assert!(s.authorize("fp-a", "secret-b").unwrap().is_none());
        assert!(s.authorize("fp-b", "secret-a").unwrap().is_none());
    }

    #[test]
    fn removing_a_device_revokes_it_but_keeps_its_counts() {
        let s = Store::in_memory().unwrap();
        s.pair("fp-a", "A", "t", 1).unwrap();
        s.add_usage("fp-a", "2026-10-13", Usage { requests: 1, tokens_in: 5, tokens_out: 9, gpu_ms: 700 }).unwrap();
        assert!(s.unpair("fp-a").unwrap());
        assert!(s.authorize("fp-a", "t").unwrap().is_none());
        assert_eq!(s.usage_since("2026-01-01").unwrap().len(), 1);
    }

    #[test]
    fn usage_adds_up_per_device_and_day() {
        let s = Store::in_memory().unwrap();
        let one = Usage { requests: 1, tokens_in: 10, tokens_out: 20, gpu_ms: 1000 };
        s.add_usage("a", "2026-10-12", one).unwrap();
        s.add_usage("a", "2026-10-13", one).unwrap();
        s.add_usage("a", "2026-10-13", one).unwrap();
        s.add_usage("b", "2026-10-13", one).unwrap();
        let mut all = s.usage_since("2026-10-13").unwrap();
        all.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(all[0], ("a".into(), Usage { requests: 2, tokens_in: 20, tokens_out: 40, gpu_ms: 2000 }));
        assert_eq!(all[1].1.requests, 1);
    }

    #[test]
    fn tokens_are_stored_hashed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.db");
        let s = Store::open(&path).unwrap();
        s.pair("fp", "A", "plain-token-value", 1).unwrap();
        drop(s);
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(17).any(|w| w == b"plain-token-value"));
    }
}
