//! The Linux D-Bus shims (`P8-T3`, `P8-T4`) against stand-ins for GNOME Shell and logind on a
//! private bus. Skipped where `dbus-daemon` isn't installed.
#![cfg(target_os = "linux")]

use std::io::{BufRead, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use host::awake::KeepAwake;
use host::linux::IdleMonitor;

struct Bus {
    child: Child,
    address: String,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn private_bus() -> Option<Bus> {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| eprintln!("skipping: no dbus-daemon ({e})"))
        .ok()?;
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take()?).read_line(&mut line).ok()?;
    Some(Bus { child, address: line.trim().to_string() })
}

struct FakeMutter {
    idle_ms: Arc<Mutex<u64>>,
}

#[zbus::interface(name = "org.gnome.Mutter.IdleMonitor")]
impl FakeMutter {
    fn get_idletime(&self) -> u64 {
        *self.idle_ms.lock().unwrap()
    }
}

#[test]
fn idle_time_comes_from_gnome_shell() {
    let Some(bus) = private_bus() else { return };
    // before GNOME is there, the rule just can't tell
    let mut mon = IdleMonitor::new(Some(bus.address.clone()));
    assert_eq!(mon.idle_secs(), None);

    let idle_ms = Arc::new(Mutex::new(125_400));
    let _shell = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.gnome.Mutter.IdleMonitor")
        .unwrap()
        .serve_at("/org/gnome/Mutter/IdleMonitor/Core", FakeMutter { idle_ms: idle_ms.clone() })
        .unwrap()
        .build()
        .unwrap();
    let mut mon = IdleMonitor::new(Some(bus.address.clone()));
    assert_eq!(mon.idle_secs(), Some(125));
    *idle_ms.lock().unwrap() = 800;
    assert_eq!(mon.idle_secs(), Some(0), "someone just touched the mouse");
}

struct FakeLogind {
    /// The write ends of the pipes handed out; writing fails once the app closes its end.
    held: Arc<Mutex<Vec<std::io::PipeWriter>>>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeLogind {
    fn inhibit(&self, what: String, _who: String, _why: String, mode: String) -> zbus::zvariant::OwnedFd {
        self.calls.lock().unwrap().push((what, mode));
        let (r, w) = std::io::pipe().unwrap();
        self.held.lock().unwrap().push(w);
        std::os::fd::OwnedFd::from(r).into()
    }
}

#[test]
fn the_computer_is_kept_awake_only_while_answering() {
    let Some(bus) = private_bus() else { return };
    let held = Arc::new(Mutex::new(vec![]));
    let calls = Arc::new(Mutex::new(vec![]));
    let _logind = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .serve_at("/org/freedesktop/login1", FakeLogind { held: held.clone(), calls: calls.clone() })
        .unwrap()
        .build()
        .unwrap();
    let still_held = || held.lock().unwrap().last_mut().map(|w| w.write_all(b"x").is_ok());

    let awake = KeepAwake::with_bus(Some(bus.address.clone()));
    awake.set(true);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(calls.lock().unwrap().as_slice(), &[("sleep".to_string(), "block".to_string())], "a blocking sleep inhibitor");
    assert_eq!(still_held(), Some(true));

    awake.set(false);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(still_held(), Some(false), "released when the answer is done");
}
