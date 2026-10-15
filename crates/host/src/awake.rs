//! Keeping the PC awake while serving (`P5-T9`, `P8-T4`, spec §6.4). Windows ties
//! `SetThreadExecutionState` to the calling thread, so one dedicated thread owns it. Linux holds
//! a logind sleep inhibitor on the same thread (D-Bus calls block).

pub struct KeepAwake {
    tx: std::sync::mpsc::Sender<bool>,
    on: std::sync::Mutex<bool>,
}

impl KeepAwake {
    pub fn new() -> Self {
        Self::with_bus(None)
    }

    /// `bus`: on Linux, a D-Bus address standing in for the system bus (tests).
    pub fn with_bus(bus: Option<String>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || {
                let mut hold = Hold::new(bus);
                for on in rx {
                    hold.set(on);
                }
                hold.set(false);
            })
            .expect("keep-awake thread");
        Self { tx, on: std::sync::Mutex::new(false) }
    }

    /// Asks the OS not to sleep (`true`) or lets it again (`false`). Only changes are sent.
    pub fn set(&self, on: bool) {
        let mut cur = self.on.lock().unwrap();
        if *cur != on {
            *cur = on;
            let _ = self.tx.send(on);
        }
    }

    pub fn is_on(&self) -> bool {
        *self.on.lock().unwrap()
    }
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)] // `bus` is only read on Linux
struct Hold {
    bus: Option<String>,
    #[cfg(target_os = "linux")]
    inhibitor: Option<crate::linux::SleepInhibitor>,
}

impl Hold {
    fn new(bus: Option<String>) -> Self {
        Self {
            bus,
            #[cfg(target_os = "linux")]
            inhibitor: None,
        }
    }

    #[cfg(windows)]
    fn set(&mut self, on: bool) {
        use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
        unsafe {
            SetThreadExecutionState(if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS });
        }
    }

    #[cfg(target_os = "linux")]
    fn set(&mut self, on: bool) {
        // dropping the inhibitor closes its file descriptor, which is what releases it
        self.inhibitor = if on { crate::linux::SleepInhibitor::take(self.bus.as_deref()) } else { None };
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn set(&mut self, _on: bool) {}
}
