//! Linux shims for Share mode (`P8-T3`, `P8-T4`), over D-Bus:
//!
//! * **Idle time** from GNOME Shell (`org.gnome.Mutter.IdleMonitor`), which is what Ubuntu's
//!   desktop runs, on Wayland and on X11. Other desktops don't offer a dependable equivalent, so
//!   there the "pause while I'm using the computer" rule can't work and the Limits screen says so.
//! * **Staying awake** while answering, with a logind "sleep" inhibitor (the Windows build uses
//!   `SetThreadExecutionState`). Releasing it is closing the file descriptor logind hands back.
//!
//! Both take an optional bus address so tests can point them at a private bus with stand-ins.

use std::time::{Duration, Instant};

use zbus::blocking::Connection;

const MUTTER: &str = "org.gnome.Mutter.IdleMonitor";
const MUTTER_PATH: &str = "/org/gnome/Mutter/IdleMonitor/Core";

/// Asks GNOME how long since the last keyboard or mouse input.
pub struct IdleMonitor {
    address: Option<String>,
    conn: Option<Connection>,
    /// Don't retry a missing session bus or desktop more than this often.
    next_try: Instant,
}

impl IdleMonitor {
    /// `address`: a D-Bus address, or `None` for the session bus.
    pub fn new(address: Option<String>) -> Self {
        Self { address, conn: None, next_try: Instant::now() }
    }

    /// Seconds since the last input, or `None` if this desktop can't tell us.
    pub fn idle_secs(&mut self) -> Option<u64> {
        if self.conn.is_none() {
            if Instant::now() < self.next_try {
                return None;
            }
            self.next_try = Instant::now() + Duration::from_secs(30);
            self.conn = connect(self.address.as_deref(), false);
        }
        let conn = self.conn.as_ref()?;
        let reply = conn.call_method(Some(MUTTER), MUTTER_PATH, Some(MUTTER), "GetIdletime", &());
        match reply.and_then(|r| r.body().deserialize::<u64>()) {
            Ok(ms) => Some(ms / 1000),
            Err(e) => {
                log::debug!("idle time not available: {e}");
                self.conn = None; // not GNOME, or the shell restarted; try again later
                None
            }
        }
    }
}

fn connect(address: Option<&str>, system: bool) -> Option<Connection> {
    let r = match address {
        Some(a) => zbus::blocking::connection::Builder::address(a).and_then(|b| b.build()),
        None if system => Connection::system(),
        None => Connection::session(),
    };
    r.map_err(|e| log::debug!("D-Bus not available: {e}")).ok()
}

/// A logind sleep inhibitor: the computer won't suspend while one is held.
pub struct SleepInhibitor {
    _fd: zbus::zvariant::OwnedFd,
}

impl SleepInhibitor {
    /// `address`: a D-Bus address, or `None` for the system bus.
    pub fn take(address: Option<&str>) -> Option<Self> {
        let conn = connect(address, true)?;
        let reply = conn
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &("sleep", "GPU Share", "Answering a friend's question", "block"),
            )
            .map_err(|e| log::warn!("couldn't keep the computer awake: {e}"))
            .ok()?;
        let fd = reply.body().deserialize::<zbus::zvariant::OwnedFd>().ok()?;
        Some(Self { _fd: fd })
    }
}
