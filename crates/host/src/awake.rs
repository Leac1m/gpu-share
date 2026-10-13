//! Keeping the PC awake while serving (`P5-T9`, spec §6.4). Windows ties
//! `SetThreadExecutionState` to the calling thread, so one dedicated thread owns it. Elsewhere
//! this does nothing (the Linux build is a client).

pub struct KeepAwake {
    tx: std::sync::mpsc::Sender<bool>,
    on: std::sync::Mutex<bool>,
}

impl KeepAwake {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || {
                for on in rx {
                    set(on);
                }
                set(false);
            })
            .expect("keep-awake thread");
        Self { tx, on: std::sync::Mutex::new(false) }
    }

    /// Asks Windows not to sleep (`true`) or lets it again (`false`). Only changes are sent.
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

#[cfg(windows)]
fn set(on: bool) {
    use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
    unsafe {
        SetThreadExecutionState(if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS });
    }
}

#[cfg(not(windows))]
fn set(_on: bool) {}
