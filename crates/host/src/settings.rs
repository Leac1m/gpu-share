//! The owner's limits (spec §6.4), persisted as JSON in the app's data folder. Field names match
//! the UI's `HostSettings` (apps/desktop/src/lib/types.ts) so the struct crosses the Tauri
//! boundary as-is.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostSettings {
    pub welcomed: bool,
    /// Whether sharing was on when the app last closed; only used if `resume_after_restart`.
    pub sharing: bool,
    pub resume_after_restart: bool,
    /// GB of VRAM the app may use.
    pub budget: f64,
    /// Context size in tokens (`-c`).
    pub ctx: u32,
    pub pause_when_in_use: bool,
    pub away_min: u32,
    pub pause_apps: Vec<String>,
    pub pause_on_gpu_use: bool,
    pub temp_pause: u32,
    pub temp_resume: u32,
    pub plugged_only: bool,
    pub hours_mode: HoursMode,
    pub hours_from: u32,
    pub hours_to: u32,
    /// Max reply length in tokens.
    pub max_reply: u32,
    /// Minutes without requests before the engine is stopped.
    pub idle_free_min: u32,
    /// Models the owner switched off.
    pub disabled: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoursMode {
    Any,
    Schedule,
}

impl Default for HostSettings {
    /// Spec §6.4 proposals; not confirmed until the feedback session (`P1-T8`). The budget is
    /// 6 GB as in the spec table and prototype build 0.2 (build 0.1 had 6.5).
    fn default() -> Self {
        Self {
            welcomed: false,
            sharing: false,
            resume_after_restart: false,
            budget: 6.0,
            ctx: 4096,
            pause_when_in_use: true,
            away_min: 5,
            pause_apps: vec!["Blender".into(), "DaVinci Resolve".into(), "Steam".into()],
            pause_on_gpu_use: true,
            temp_pause: 83,
            temp_resume: 75,
            plugged_only: true,
            hours_mode: HoursMode::Any,
            hours_from: 19,
            hours_to: 8,
            max_reply: 1024,
            idle_free_min: 5,
            disabled: vec![],
        }
    }
}

impl HostSettings {
    /// Keeps every value inside the range the UI offers, whatever arrives from a file or the
    /// webview.
    pub fn sanitized(mut self) -> Self {
        self.budget = if self.budget.is_finite() { self.budget.clamp(1.0, 256.0) } else { 6.0 };
        self.ctx = self.ctx.clamp(512, 32768);
        self.away_min = self.away_min.clamp(1, 120);
        self.temp_pause = self.temp_pause.clamp(60, 95);
        self.temp_resume = self.temp_resume.min(self.temp_pause.saturating_sub(1));
        self.hours_from %= 24;
        self.hours_to %= 24;
        self.max_reply = self.max_reply.clamp(16, 8192);
        self.idle_free_min = self.idle_free_min.clamp(1, 240);
        self.pause_apps.retain(|a| !a.trim().is_empty());
        self
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<HostSettings>(&s).ok())
            .unwrap_or_default()
            .sanitized()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap())?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_matches_the_ui() {
        let v = serde_json::to_value(HostSettings::default()).unwrap();
        for k in ["pauseWhenInUse", "awayMin", "maxReply", "hoursMode", "resumeAfterRestart", "idleFreeMin"] {
            assert!(v.get(k).is_some(), "{k}");
        }
        assert_eq!(v["hoursMode"], "any");
    }

    #[test]
    fn missing_or_odd_values_fall_back_safely() {
        let s: HostSettings = serde_json::from_str(r#"{"maxReply": 999999, "tempPause": 80, "tempResume": 90}"#).unwrap();
        let s = s.sanitized();
        assert_eq!(s.max_reply, 8192);
        assert!(s.temp_resume < s.temp_pause, "resume must be below pause or it flaps");
        assert_eq!(s.ctx, 4096, "missing fields take the defaults");
    }

    #[test]
    fn saves_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        let s = HostSettings { budget: 5.5, ..Default::default() };
        s.save(&p).unwrap();
        assert_eq!(HostSettings::load(&p).budget, 5.5);
        assert_eq!(HostSettings::load(&dir.path().join("missing.json")), HostSettings::default());
    }
}
