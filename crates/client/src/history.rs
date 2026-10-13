//! The client's SQLite file (spec §6.9): chat history and the hosts this computer is paired
//! with. History lives only here; nothing is kept on the host.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
#[error("client database: {0}")]
pub struct HistoryError(#[from] rusqlite::Error);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub role: String,
    pub text: String,
}

/// A host this computer paired with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHost {
    /// Certificate fingerprint.
    pub id: String,
    pub name: String,
    pub token: String,
    /// Last address it was reached at, "ip:port".
    pub address: String,
}

pub struct History {
    db: Mutex<Connection>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS messages (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    host_id TEXT NOT NULL,
    role    TEXT NOT NULL,          -- user | assistant | note
    text    TEXT NOT NULL,
    at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_host ON messages (host_id, id);
CREATE TABLE IF NOT EXISTS hosts (
    id      TEXT PRIMARY KEY,       -- certificate fingerprint
    name    TEXT NOT NULL,
    token   TEXT NOT NULL,
    address TEXT NOT NULL
);
";

impl History {
    pub fn open(path: &Path) -> Result<Self, HistoryError> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, HistoryError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, HistoryError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { db: Mutex::new(conn) })
    }

    pub fn add(&self, host_id: &str, role: &str, text: &str, at: i64) -> Result<(), HistoryError> {
        self.db.lock().unwrap().execute(
            "INSERT INTO messages (host_id, role, text, at) VALUES (?1, ?2, ?3, ?4)",
            params![host_id, role, text, at],
        )?;
        Ok(())
    }

    /// The last `limit` messages with a host, oldest first.
    pub fn recent(&self, host_id: &str, limit: usize) -> Result<Vec<Message>, HistoryError> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(
            "SELECT role, text FROM (SELECT id, role, text FROM messages WHERE host_id = ?1 ORDER BY id DESC LIMIT ?2)
             ORDER BY id",
        )?;
        let rows = st.query_map(params![host_id, limit as i64], |r| Ok(Message { role: r.get(0)?, text: r.get(1)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn clear(&self, host_id: &str) -> Result<(), HistoryError> {
        self.db.lock().unwrap().execute("DELETE FROM messages WHERE host_id = ?1", params![host_id])?;
        Ok(())
    }

    pub fn save_host(&self, h: &KnownHost) -> Result<(), HistoryError> {
        self.db.lock().unwrap().execute(
            "INSERT INTO hosts (id, name, token, address) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, token = excluded.token, address = excluded.address",
            params![h.id, h.name, h.token, h.address],
        )?;
        Ok(())
    }

    pub fn update_address(&self, id: &str, name: &str, address: &str) -> Result<(), HistoryError> {
        self.db
            .lock()
            .unwrap()
            .execute("UPDATE hosts SET name = ?2, address = ?3 WHERE id = ?1", params![id, name, address])?;
        Ok(())
    }

    pub fn host(&self, id: &str) -> Result<Option<KnownHost>, HistoryError> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id, name, token, address FROM hosts WHERE id = ?1", params![id], |r| {
                Ok(KnownHost { id: r.get(0)?, name: r.get(1)?, token: r.get(2)?, address: r.get(3)? })
            })
            .optional()?)
    }

    pub fn hosts(&self) -> Result<Vec<KnownHost>, HistoryError> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare("SELECT id, name, token, address FROM hosts ORDER BY name")?;
        let rows = st.query_map([], |r| {
            Ok(KnownHost { id: r.get(0)?, name: r.get(1)?, token: r.get(2)?, address: r.get(3)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn forget_host(&self, id: &str) -> Result<(), HistoryError> {
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM hosts WHERE id = ?1", params![id])?;
        db.execute("DELETE FROM messages WHERE host_id = ?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_returns_the_tail_in_order() {
        let h = History::in_memory().unwrap();
        for i in 0..5 {
            h.add("a", "user", &format!("m{i}"), i).unwrap();
        }
        h.add("b", "user", "other host", 9).unwrap();
        let r: Vec<String> = h.recent("a", 3).unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(r, vec!["m2", "m3", "m4"]);
    }

    #[test]
    fn forgetting_a_host_removes_its_token_and_history() {
        let h = History::in_memory().unwrap();
        h.save_host(&KnownHost { id: "a".into(), name: "A".into(), token: "t".into(), address: "1.2.3.4:50123".into() })
            .unwrap();
        h.add("a", "user", "hi", 1).unwrap();
        h.forget_host("a").unwrap();
        assert!(h.host("a").unwrap().is_none());
        assert!(h.recent("a", 10).unwrap().is_empty());
    }
}
