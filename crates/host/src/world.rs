//! Reading what the computer is doing (`P5-T2`–`P5-T6`) into [`Signals`] for the policy loop.
//!
//! Each OS shim is small and lives behind [`World`], so the decision itself stays pure and
//! tested (`policy.rs`). The shims are verified by hand on real machines (roadmap manual
//! checklist); CI only compiles the Windows ones.
//!
//! | Signal | Windows | Linux |
//! |---|---|---|
//! | idle time | `GetLastInputInfo` | not available (rule stays inactive) |
//! | running apps | process list (`sysinfo`) | same |
//! | other app on the GPU | GPU Engine performance counters | not available |
//! | temperature, VRAM | NVML (driver's `nvml.dll`) | NVML (`libnvidia-ml.so`) if present |
//! | battery | `GetSystemPowerStatus` | `/sys/class/power_supply` |

use std::path::PathBuf;
use std::sync::Mutex;

use crate::policy::Signals;

pub trait World: Send + Sync + 'static {
    /// Takes a fresh reading. May block for a few milliseconds; called off the async threads.
    fn sample(&self) -> Signals;
    /// The GPU's name and total VRAM in GB, if there is one this app can see.
    fn card(&self) -> Option<(String, f64)> {
        None
    }
    /// Free VRAM right now, in GB (checked just before loading a model, `P5-T5`).
    fn free_vram_gb(&self) -> Option<f64> {
        None
    }
    /// Process ids that are ours (the app and the engine), never counted as "another app".
    fn set_own_pids(&self, _pids: Vec<u32>) {}
}

/// Friendly names for the programs people are likely to list (roadmap D6), by executable.
pub const KNOWN_APPS: &[(&str, &str)] = &[
    ("blender", "Blender"),
    ("resolve", "DaVinci Resolve"),
    ("photoshop", "Photoshop"),
    ("adobe premiere pro", "Premiere Pro"),
    ("afterfx", "After Effects"),
    ("unrealeditor", "Unreal Engine"),
    ("ue4editor", "Unreal Engine"),
    ("unity", "Unity"),
    ("cinema 4d", "Cinema 4D"),
    ("steam", "Steam"),
    ("obs64", "OBS Studio"),
    ("obs", "OBS Studio"),
];

/// Background processes nobody would put on a pause list.
const SYSTEM: &[&str] = &[
    "svchost", "system", "registry", "smss", "csrss", "wininit", "services", "lsass", "winlogon", "dwm", "fontdrvhost",
    "conhost", "runtimebroker", "sihost", "taskhostw", "ctfmon", "searchhost", "startmenuexperiencehost",
    "textinputhost", "shellexperiencehost", "dllhost", "audiodg", "spoolsv", "wmiprvse", "explorer", "smartscreen",
    "securityhealthsystray", "securityhealthservice", "msmpeng", "nissrv", "systemd", "kworker", "bash", "sh", "zsh",
    "dbus-daemon", "pipewire", "wireplumber", "pulseaudio", "xorg", "xwayland", "gnome-shell", "llama-server",
    "app", "gpu share", "gpu-share",
];

/// "Blender.exe" → "Blender"; anything unknown keeps its executable name without `.exe`.
pub fn friendly_name(exe: &str) -> String {
    let stem = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(exe);
    let low = stem.to_ascii_lowercase();
    KNOWN_APPS.iter().find(|(e, _)| *e == low).map(|(_, f)| f.to_string()).unwrap_or_else(|| stem.to_string())
}

/// Distinct, user-visible program names from raw process names, known apps first.
pub fn app_list(processes: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut known = vec![];
    let mut other = vec![];
    for p in processes {
        let stem = p.strip_suffix(".exe").unwrap_or(&p).to_ascii_lowercase();
        if SYSTEM.contains(&stem.as_str()) || stem.is_empty() {
            continue;
        }
        let name = friendly_name(&p);
        let bucket = if KNOWN_APPS.iter().any(|(_, f)| *f == name) { &mut known } else { &mut other };
        if !bucket.contains(&name) {
            bucket.push(name);
        }
    }
    other.sort_by_key(|n| n.to_ascii_lowercase());
    known.extend(other);
    known
}

fn local_hour() -> u32 {
    use chrono::Timelike;
    chrono::Local::now().hour()
}

// --------------------------------------------------------------------------- the real machine

pub struct SystemWorld {
    procs: Mutex<sysinfo::System>,
    nvml: Option<nvml_wrapper::Nvml>,
    own_pids: Mutex<Vec<u32>>,
    #[cfg(windows)]
    gpu_counters: Mutex<Option<win::GpuCounters>>,
}

impl SystemWorld {
    pub fn new() -> Self {
        let nvml = nvml_wrapper::Nvml::init().map_err(|e| log::info!("NVML not available: {e}")).ok();
        Self {
            procs: Mutex::new(sysinfo::System::new()),
            nvml,
            own_pids: Mutex::new(vec![std::process::id()]),
            #[cfg(windows)]
            gpu_counters: Mutex::new(win::GpuCounters::open().map_err(|e| log::warn!("GPU counters: {e}")).ok()),
        }
    }

    fn running_apps(&self) -> (Vec<String>, std::collections::HashMap<u32, String>) {
        let mut sys = self.procs.lock().unwrap();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let me = sysinfo::get_current_pid().ok().and_then(|p| sys.process(p)).and_then(|p| p.user_id().cloned());
        let mut names = vec![];
        let mut by_pid = std::collections::HashMap::new();
        for (pid, p) in sys.processes() {
            let name = p.name().to_string_lossy().to_string();
            by_pid.insert(pid.as_u32(), name.clone());
            // only the owner's own programs (not services, not other users')
            if me.is_some() && p.user_id().cloned() != me {
                continue;
            }
            names.push(name);
        }
        (app_list(names), by_pid)
    }

    fn temp(&self) -> Option<f64> {
        let dev = self.nvml.as_ref()?.device_by_index(0).ok()?;
        dev.temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu).ok().map(|t| t as f64)
    }
}

impl Default for SystemWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl World for SystemWorld {
    fn sample(&self) -> Signals {
        let (running_apps, by_pid) = self.running_apps();
        #[cfg(windows)]
        let other_gpu_user = {
            let own = self.own_pids.lock().unwrap().clone();
            self.gpu_counters
                .lock()
                .unwrap()
                .as_mut()
                .and_then(|c| c.busiest_other(&own))
                .map(|pid| by_pid.get(&pid).map(|n| friendly_name(n)).unwrap_or_else(|| "Another program".into()))
        };
        #[cfg(not(windows))]
        let other_gpu_user = {
            let _ = by_pid;
            None
        };
        Signals {
            idle_secs: idle_secs(),
            running_apps,
            other_gpu_user,
            gpu_temp_c: self.temp(),
            on_battery: on_battery(),
            hour: local_hour(),
        }
    }

    fn card(&self) -> Option<(String, f64)> {
        let dev = self.nvml.as_ref()?.device_by_index(0).ok()?;
        let name = dev.name().ok()?.replace("NVIDIA ", "");
        let total = dev.memory_info().ok()?.total as f64 / (1024.0 * 1024.0 * 1024.0);
        Some((format!("{name} {:.0} GB", total), total))
    }

    fn free_vram_gb(&self) -> Option<f64> {
        let dev = self.nvml.as_ref()?.device_by_index(0).ok()?;
        Some(dev.memory_info().ok()?.free as f64 / (1024.0 * 1024.0 * 1024.0))
    }

    fn set_own_pids(&self, pids: Vec<u32>) {
        let mut own = self.own_pids.lock().unwrap();
        *own = pids;
        own.push(std::process::id());
    }
}

#[cfg(windows)]
fn idle_secs() -> Option<u64> {
    win::idle_secs()
}

#[cfg(not(windows))]
fn idle_secs() -> Option<u64> {
    None
}

#[cfg(windows)]
fn on_battery() -> Option<bool> {
    win::on_battery()
}

/// Linux: on battery when a battery is discharging and no mains supply is online.
#[cfg(not(windows))]
fn on_battery() -> Option<bool> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut has_battery = false;
    for e in dir.flatten() {
        let p = e.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        match kind.trim() {
            "Mains" if std::fs::read_to_string(p.join("online")).unwrap_or_default().trim() == "1" => return Some(false),
            "Battery" => has_battery = true,
            _ => {}
        }
    }
    has_battery.then_some(true)
}

// --------------------------------------------------------------------------- Windows shims

#[cfg(windows)]
mod win {
    use windows::core::w;
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
        PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    };
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    /// Seconds since the last keyboard or mouse input in this session (`P5-T2`).
    pub fn idle_secs() -> Option<u64> {
        let mut lii = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        unsafe {
            if !GetLastInputInfo(&mut lii).as_bool() {
                return None;
            }
            Some(GetTickCount().wrapping_sub(lii.dwTime) as u64 / 1000)
        }
    }

    /// `P5-T6`. `ACLineStatus` 0 = on battery, 1 = plugged in, 255 = unknown (desktops).
    pub fn on_battery() -> Option<bool> {
        let mut s = SYSTEM_POWER_STATUS::default();
        unsafe { GetSystemPowerStatus(&mut s).ok()? };
        match s.ACLineStatus {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        }
    }

    /// Utilization above this (percent, summed over a process's engines) counts as "using the GPU".
    const BUSY_PERCENT: f64 = 15.0;

    /// The "GPU Engine(*)\Utilization Percentage" counters Task Manager shows (`P5-T4`). NVML
    /// often can't attribute GPU use to processes on Windows (WDDM), these can.
    pub struct GpuCounters {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
    }

    unsafe impl Send for GpuCounters {}

    impl GpuCounters {
        pub fn open() -> Result<Self, String> {
            unsafe {
                let mut query = PDH_HQUERY::default();
                let r = PdhOpenQueryW(None, 0, &mut query);
                if r != 0 {
                    return Err(format!("PdhOpenQuery {r:#x}"));
                }
                let mut counter = PDH_HCOUNTER::default();
                let r = PdhAddEnglishCounterW(query, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut counter);
                if r != 0 {
                    PdhCloseQuery(query);
                    return Err(format!("PdhAddEnglishCounter {r:#x}"));
                }
                PdhCollectQueryData(query); // rates need two samples; the first is a baseline
                Ok(Self { query, counter })
            }
        }

        /// The process (not ours) using the GPU most, if it's above the threshold.
        pub fn busiest_other(&mut self, own: &[u32]) -> Option<u32> {
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
                let mut size = 0u32;
                let mut count = 0u32;
                let r = PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
                if r != PDH_MORE_DATA as u32 || size == 0 {
                    return None;
                }
                let mut buf = vec![0u8; size as usize];
                let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
                if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
                    return None;
                }
                let mut per_pid: std::collections::HashMap<u32, f64> = Default::default();
                for i in 0..count as usize {
                    let item = &*items.add(i);
                    let name = item.szName.to_string().unwrap_or_default();
                    // instance names look like "pid_1234_luid_0x..._phys_0_eng_0_engtype_3D"
                    let Some(pid) = name.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse().ok())
                    else {
                        continue;
                    };
                    *per_pid.entry(pid).or_default() += item.FmtValue.Anonymous.doubleValue;
                }
                per_pid
                    .into_iter()
                    .filter(|(pid, _)| !own.contains(pid))
                    .filter(|(_, pct)| *pct >= BUSY_PERCENT)
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(pid, _)| pid)
            }
        }
    }

    impl Drop for GpuCounters {
        fn drop(&mut self) {
            unsafe {
                PdhCloseQuery(self.query);
            }
        }
    }
}

// --------------------------------------------------------------------------- dev / tests

/// Signals read from a JSON file on every tick (`GPU_SHARE_WORLD_FILE`). Lets the pause rules be
/// exercised in the real app on a machine without the real signals, e.g.
/// `{"running_apps": ["Blender"], "gpu_temp_c": 85}`. Missing fields read as "can't tell".
pub struct FileWorld {
    pub path: PathBuf,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct FileSignals {
    idle_secs: Option<u64>,
    running_apps: Vec<String>,
    other_gpu_user: Option<String>,
    gpu_temp_c: Option<f64>,
    on_battery: Option<bool>,
    hour: Option<u32>,
    card: Option<(String, f64)>,
    free_vram_gb: Option<f64>,
}

impl FileWorld {
    fn read(&self) -> FileSignals {
        std::fs::read_to_string(&self.path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
}

impl World for FileWorld {
    fn sample(&self) -> Signals {
        let f = self.read();
        Signals {
            idle_secs: f.idle_secs,
            running_apps: f.running_apps,
            other_gpu_user: f.other_gpu_user,
            gpu_temp_c: f.gpu_temp_c,
            on_battery: f.on_battery,
            hour: f.hour.unwrap_or_else(local_hour),
        }
    }

    fn card(&self) -> Option<(String, f64)> {
        self.read().card
    }

    fn free_vram_gb(&self) -> Option<f64> {
        self.read().free_vram_gb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_programs_get_friendly_names() {
        assert_eq!(friendly_name("blender.exe"), "Blender");
        assert_eq!(friendly_name("Resolve.exe"), "DaVinci Resolve");
        assert_eq!(friendly_name("obs64.exe"), "OBS Studio");
        assert_eq!(friendly_name("MyGame.exe"), "MyGame");
        assert_eq!(friendly_name("blender"), "Blender");
    }

    #[test]
    fn the_app_list_is_deduplicated_known_first_and_without_system_processes() {
        let list = app_list(
            ["svchost.exe", "zoom.exe", "blender.exe", "Blender.exe", "steam.exe", "chrome.exe", "chrome.exe", "dwm.exe", "explorer.exe"]
                .map(String::from),
        );
        assert_eq!(list, vec!["Blender", "Steam", "chrome", "zoom"]);
    }

    #[test]
    fn the_file_world_reads_what_it_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("world.json");
        std::fs::write(&path, r#"{"running_apps": ["Blender"], "gpu_temp_c": 85, "hour": 3}"#).unwrap();
        let w = FileWorld { path };
        let s = w.sample();
        assert_eq!(s.running_apps, vec!["Blender"]);
        assert_eq!(s.gpu_temp_c, Some(85.0));
        assert_eq!(s.hour, 3);
        assert_eq!(s.on_battery, None, "missing means can't tell");
    }

    #[test]
    fn the_system_world_samples_without_panicking() {
        let w = SystemWorld::new();
        let s = w.sample();
        assert!(s.hour < 24);
        assert!(!s.running_apps.is_empty() || cfg!(windows), "at least the test runner's own shell-less processes");
    }
}
