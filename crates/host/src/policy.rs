//! The policy loop's decision (`P5-T1`, spec §6.3): from the owner's settings and what the
//! computer is doing right now, is sharing off, paused (and why), or open?
//!
//! Pure: no clocks, no OS calls. [`crate::world`] gathers the [`Signals`]; this decides. The
//! precedence is the prototype's `policy()` (docs/gpu-share-preview.html), and the table of
//! tests below is its executable form. The owner sees the exact reason; friends see a general
//! one that never names a program (spec §6.3).

use crate::core::Gate;
use crate::settings::{HostSettings, HoursMode};

/// What the computer is doing. `None` means "can't tell on this machine", and a rule that can't
/// tell never pauses sharing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signals {
    /// Seconds since the owner last touched the keyboard or mouse.
    pub idle_secs: Option<u64>,
    /// Names of running programs (friendly names where known, else the executable's name).
    pub running_apps: Vec<String>,
    /// A program other than ours is using the GPU, and its name.
    pub other_gpu_user: Option<String>,
    pub gpu_temp_c: Option<f64>,
    pub on_battery: Option<bool>,
    /// Local hour of day, 0–23.
    pub hour: u32,
}

/// Remembered between ticks: the temperature rule's hysteresis.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PolicyState {
    pub too_hot: bool,
}

/// Below this many idle seconds the owner counts as "using the computer" (no countdown yet).
pub const AT_DESK_SECS: u64 = 30;

pub const IN_USE: &str = "They're using their computer";

fn paused(owner: impl Into<String>, friend: impl Into<String>) -> Gate {
    Gate::Paused { owner: owner.into(), friend: friend.into(), countdown: false }
}

pub fn fmt_hour(h: u32) -> String {
    let h12 = if h.is_multiple_of(12) { 12 } else { h % 12 };
    format!("{h12}{}", if h < 12 { "am" } else { "pm" })
}

fn in_hours(from: u32, to: u32, h: u32) -> bool {
    if from <= to {
        h >= from && h < to
    } else {
        h >= from || h < to
    }
}

/// Case-insensitive match of a listed app against what's running.
fn listed_and_running<'a>(list: &'a [String], running: &[String]) -> Option<&'a String> {
    list.iter().find(|a| running.iter().any(|r| r.eq_ignore_ascii_case(a)))
}

pub fn decide(s: &HostSettings, sig: &Signals, state: &mut PolicyState) -> Gate {
    // temperature hysteresis runs every tick, so it's right the moment it matters
    if let Some(t) = sig.gpu_temp_c {
        if t >= s.temp_pause as f64 {
            state.too_hot = true;
        } else if t <= s.temp_resume as f64 {
            state.too_hot = false;
        }
    } else {
        state.too_hot = false;
    }

    if !s.sharing {
        return Gate::Off;
    }
    if s.plugged_only && sig.on_battery == Some(true) {
        return paused("Your laptop is running on battery", "Their laptop is on battery");
    }
    if s.hours_mode == HoursMode::Schedule && !in_hours(s.hours_from, s.hours_to, sig.hour) {
        return paused(
            format!("Outside your sharing hours, {} to {}", fmt_hour(s.hours_from), fmt_hour(s.hours_to)),
            "Outside the hours they share",
        );
    }
    if let Some(app) = listed_and_running(&s.pause_apps, &sig.running_apps) {
        return paused(format!("{app} is open"), IN_USE);
    }
    if s.pause_on_gpu_use {
        if let Some(app) = &sig.other_gpu_user {
            return paused(format!("{app} is using your graphics card"), IN_USE);
        }
    }
    if s.pause_when_in_use {
        if let Some(idle) = sig.idle_secs {
            let away = s.away_min as u64 * 60;
            if idle < AT_DESK_SECS {
                return paused("You're using your computer", IN_USE);
            }
            if idle < away {
                let n = (away - idle).div_ceil(60);
                return Gate::Paused {
                    owner: format!("Starts again in {n} min"),
                    friend: format!("They just stepped away. Back in about {n} min"),
                    countdown: true,
                };
            }
        }
    }
    if state.too_hot {
        let t = sig.gpu_temp_c.unwrap_or_default().round();
        return paused(format!("Your graphics card is at {t}°C"), "Their graphics card is cooling down");
    }
    Gate::Open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> HostSettings {
        HostSettings { sharing: true, ..Default::default() }
    }

    /// Someone who has been away a long time, plugged in, cool, at 3pm: sharing is open.
    fn calm() -> Signals {
        Signals { idle_secs: Some(3600), running_apps: vec![], other_gpu_user: None, gpu_temp_c: Some(50.0), on_battery: Some(false), hour: 15 }
    }

    fn owner_reason(g: &Gate) -> String {
        match g {
            Gate::Paused { owner, .. } => owner.clone(),
            Gate::Open => "open".into(),
            Gate::Off => "off".into(),
        }
    }

    fn friend_reason(g: &Gate) -> String {
        match g {
            Gate::Paused { friend, .. } => friend.clone(),
            other => owner_reason(other),
        }
    }

    fn run(s: &HostSettings, sig: &Signals) -> Gate {
        decide(s, sig, &mut PolicyState::default())
    }

    #[test]
    fn the_table() {
        // (what changes, owner sees, friends see)
        type Case = (&'static str, fn(&mut HostSettings, &mut Signals), &'static str, &'static str);
        let cases: Vec<Case> = vec![
            ("nothing", |_, _| {}, "open", "open"),
            ("switch off", |s, _| s.sharing = false, "off", "off"),
            ("switch off beats everything", |s, g| { s.sharing = false; g.on_battery = Some(true); g.running_apps = vec!["Blender".into()]; }, "off", "off"),
            ("on battery", |_, g| g.on_battery = Some(true), "Your laptop is running on battery", "Their laptop is on battery"),
            ("on battery, rule off", |s, g| { s.plugged_only = false; g.on_battery = Some(true); }, "open", "open"),
            ("battery unknown (desktop)", |_, g| g.on_battery = None, "open", "open"),
            ("battery beats schedule", |s, g| { g.on_battery = Some(true); s.hours_mode = HoursMode::Schedule; }, "Your laptop is running on battery", "Their laptop is on battery"),
            ("outside hours", |s, _| s.hours_mode = HoursMode::Schedule, "Outside your sharing hours, 7pm to 8am", "Outside the hours they share"),
            ("inside overnight hours, late", |s, g| { s.hours_mode = HoursMode::Schedule; g.hour = 23; }, "open", "open"),
            ("inside overnight hours, early", |s, g| { s.hours_mode = HoursMode::Schedule; g.hour = 2; }, "open", "open"),
            ("end hour is exclusive", |s, g| { s.hours_mode = HoursMode::Schedule; g.hour = 8; }, "Outside your sharing hours, 7pm to 8am", "Outside the hours they share"),
            ("daytime window", |s, g| { s.hours_mode = HoursMode::Schedule; s.hours_from = 9; s.hours_to = 17; g.hour = 12; }, "open", "open"),
            ("schedule beats apps", |s, g| { s.hours_mode = HoursMode::Schedule; g.running_apps = vec!["Blender".into()]; }, "Outside your sharing hours, 7pm to 8am", "Outside the hours they share"),
            ("listed app open", |_, g| g.running_apps = vec!["Blender".into()], "Blender is open", IN_USE),
            ("listed app, any case", |_, g| g.running_apps = vec!["blender".into()], "Blender is open", IN_USE),
            ("unlisted app", |_, g| g.running_apps = vec!["Chrome".into()], "open", "open"),
            ("first listed app wins", |_, g| g.running_apps = vec!["Steam".into(), "Blender".into()], "Blender is open", IN_USE),
            ("listed app beats GPU use", |_, g| { g.running_apps = vec!["Steam".into()]; g.other_gpu_user = Some("Game.exe".into()); }, "Steam is open", IN_USE),
            ("other GPU user", |_, g| g.other_gpu_user = Some("Cyberpunk2077".into()), "Cyberpunk2077 is using your graphics card", IN_USE),
            ("other GPU user, rule off", |s, g| { s.pause_on_gpu_use = false; g.other_gpu_user = Some("X".into()); }, "open", "open"),
            ("GPU use beats the desk", |_, g| { g.other_gpu_user = Some("X".into()); g.idle_secs = Some(0); }, "X is using your graphics card", IN_USE),
            ("at the desk", |_, g| g.idle_secs = Some(3), "You're using your computer", IN_USE),
            ("at the desk, rule off", |s, g| { s.pause_when_in_use = false; g.idle_secs = Some(3); }, "open", "open"),
            ("just stepped away", |_, g| g.idle_secs = Some(60), "Starts again in 4 min", "They just stepped away. Back in about 4 min"),
            ("almost back", |_, g| g.idle_secs = Some(299), "Starts again in 1 min", "They just stepped away. Back in about 1 min"),
            ("away long enough", |_, g| g.idle_secs = Some(300), "open", "open"),
            ("longer away setting", |s, g| { s.away_min = 15; g.idle_secs = Some(600); }, "Starts again in 5 min", "They just stepped away. Back in about 5 min"),
            ("idle unknown (Linux)", |_, g| g.idle_secs = None, "open", "open"),
            ("desk beats heat", |_, g| { g.idle_secs = Some(1); g.gpu_temp_c = Some(90.0); }, "You're using your computer", IN_USE),
            ("hot", |_, g| g.gpu_temp_c = Some(83.0), "Your graphics card is at 83°C", "Their graphics card is cooling down"),
            ("warm but under the limit", |_, g| g.gpu_temp_c = Some(82.4), "open", "open"),
            ("temperature unknown", |_, g| g.gpu_temp_c = None, "open", "open"),
            ("lower limit set", |s, g| { s.temp_pause = 70; s.temp_resume = 62; g.gpu_temp_c = Some(71.0); }, "Your graphics card is at 71°C", "Their graphics card is cooling down"),
        ];
        for (name, change, owner, friend) in cases {
            let mut s = on();
            let mut g = calm();
            change(&mut s, &mut g);
            let gate = run(&s, &g);
            assert_eq!(owner_reason(&gate), owner, "{name}: owner");
            assert_eq!(friend_reason(&gate), friend, "{name}: friends");
        }
    }

    #[test]
    fn friends_never_see_a_program_name() {
        let apps = ["Blender", "DaVinci Resolve", "Steam", "Photoshop"];
        for app in apps {
            let mut g = calm();
            g.running_apps = vec![app.into()];
            g.other_gpu_user = Some(app.into());
            let mut s = on();
            s.pause_apps = apps.iter().map(|a| a.to_string()).collect();
            let friend = friend_reason(&run(&s, &g));
            assert!(!friend.contains(app), "{friend}");
            s.pause_apps.clear();
            let friend = friend_reason(&run(&s, &g));
            assert!(!friend.contains(app), "{friend}");
        }
    }

    #[test]
    fn temperature_has_hysteresis_and_does_not_flap() {
        let s = on();
        let mut st = PolicyState::default();
        let at = |t: f64| Signals { gpu_temp_c: Some(t), ..calm() };
        let seq = [70.0, 82.0, 83.0, 80.0, 78.0, 76.0, 75.5, 75.0, 79.0, 82.9, 83.5];
        let got: Vec<bool> = seq.iter().map(|&t| matches!(decide(&s, &at(t), &mut st), Gate::Paused { .. })).collect();
        assert_eq!(got, vec![false, false, true, true, true, true, true, false, false, false, true]);
    }

    #[test]
    fn heat_is_tracked_even_while_paused_for_something_else() {
        let s = on();
        let mut st = PolicyState::default();
        let mut g = Signals { gpu_temp_c: Some(90.0), idle_secs: Some(0), ..calm() };
        decide(&s, &g, &mut st);
        assert!(st.too_hot, "remembered while the desk rule was the visible reason");
        g.idle_secs = Some(3600);
        g.gpu_temp_c = Some(80.0);
        assert!(owner_reason(&decide(&s, &g, &mut st)).contains("80°C"), "still cooling down");
    }

    #[test]
    fn the_countdown_flag_is_set_only_for_the_away_timer() {
        let mut g = calm();
        g.idle_secs = Some(100);
        assert!(matches!(run(&on(), &g), Gate::Paused { countdown: true, .. }));
        g.idle_secs = Some(1);
        assert!(matches!(run(&on(), &g), Gate::Paused { countdown: false, .. }));
    }

    #[test]
    fn hours_format_like_the_prototype() {
        assert_eq!(fmt_hour(0), "12am");
        assert_eq!(fmt_hour(8), "8am");
        assert_eq!(fmt_hour(12), "12pm");
        assert_eq!(fmt_hour(19), "7pm");
    }
}
