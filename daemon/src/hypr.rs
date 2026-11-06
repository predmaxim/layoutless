//! Hyprland IPC: layout switching (socket1) and layout/focus events (socket2).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::daemon::Msg;

fn socket_dir() -> std::io::Result<PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(std::io::Error::other)?;
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").map_err(std::io::Error::other)?;
    Ok(PathBuf::from(runtime).join("hypr").join(sig))
}

/// Run a raw socket1 command and return the reply.
pub fn command(cmd: &str) -> std::io::Result<String> {
    let mut s = UnixStream::connect(socket_dir()?.join(".socket.sock"))?;
    s.write_all(cmd.as_bytes())?;
    let mut reply = String::new();
    s.read_to_string(&mut reply)?;
    Ok(reply)
}

pub fn switch_layout(device: &str, index: usize) -> std::io::Result<()> {
    let reply = command(&format!("switchxkblayout {device} {index}"))?;
    if reply.trim() != "ok" {
        return Err(std::io::Error::other(format!("switchxkblayout: {reply}")));
    }
    Ok(())
}

/// Run Lua in Hyprland's config context (`hyprctl eval`).
pub fn eval(code: &str) -> std::io::Result<()> {
    let reply = command(&format!("eval {code}"))?;
    if reply.trim() != "ok" {
        return Err(std::io::Error::other(format!("eval: {}", reply.trim())));
    }
    Ok(())
}

/// Address of the focused window as in events (no `0x`), "" if none.
pub fn active_address() -> std::io::Result<String> {
    let reply = command("j/activewindow")?;
    let v: serde_json::Value = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
    Ok(v["address"].as_str().unwrap_or("").trim_start_matches("0x").to_string())
}

/// Class of the focused window ("" if none).
pub fn active_class() -> std::io::Result<String> {
    let reply = command("j/activewindow")?;
    let v: serde_json::Value = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
    Ok(v["class"].as_str().unwrap_or("").to_string())
}

/// Whether a session lock (the lock screen) is active.
pub fn is_locked() -> std::io::Result<bool> {
    let reply = command("j/locked")?;
    let v: serde_json::Value = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
    v["locked"].as_bool().ok_or_else(|| std::io::Error::other(format!("locked: {reply}")))
}

/// Hyprland's key repeat delay. Apps repeat a held key themselves after it,
/// and the daemon never sees those repeats.
pub fn repeat_delay() -> std::io::Result<Duration> {
    let reply = command("j/getoption input:repeat_delay")?;
    let v: serde_json::Value = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
    let ms = v["int"].as_u64().ok_or_else(|| std::io::Error::other(format!("repeat_delay: {reply}")))?;
    Ok(Duration::from_millis(ms))
}

struct Snapshot {
    /// The real keyboard the current layout is read from.
    reference: String,
    reference_index: usize,
    watched_index: Option<usize>,
}

/// Read layout indices from `hyprctl devices`.
///
/// Don't use the keyboard flagged `main`: with fcitx5 running, `main` is its
/// virtual keyboard, which `switchxkblayout all` doesn't touch. Don't use our
/// own virtual keyboard either: it starts at index 0, and `switchxkblayout all
/// next` cycles each device separately, so it can be out of sync.
fn snapshot(watched: &str) -> std::io::Result<Snapshot> {
    let reply = command("j/devices")?;
    let v: serde_json::Value = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
    let kbds = v["keyboards"].as_array().cloned().unwrap_or_default();
    let index = |k: &serde_json::Value| k["active_layout_index"].as_u64().unwrap_or(0) as usize;
    let reference = kbds
        .iter()
        .find(|k| {
            let name = k["name"].as_str().unwrap_or("");
            !name.contains("virtual") && name.ends_with("keyboard")
        })
        .ok_or_else(|| std::io::Error::other("no real keyboard found"))?;
    Ok(Snapshot {
        reference: reference["name"].as_str().unwrap_or("").to_string(),
        reference_index: index(reference),
        watched_index: kbds.iter().find(|k| k["name"] == watched).map(index),
    })
}

/// The layout to give every keyboard when `device` is on another one than the
/// reference: a group switch (Alt+Alt) changes only the keyboard it was typed
/// on, so with an external keyboard the reference never saw it.
fn follow_index(kbds: &[serde_json::Value], device: &str, reference: &str) -> Option<usize> {
    let index = |name: &str| kbds.iter().find(|k| k["name"] == name).and_then(|k| k["active_layout_index"].as_u64());
    match (index(device), index(reference)) {
        (Some(d), Some(r)) if d != r => Some(d as usize),
        _ => None,
    }
}

/// Bring every keyboard to `device`'s layout; the reference's own event then
/// reports the change as usual.
fn follow(device: &str, reference: &str) -> std::io::Result<()> {
    let reply = command("j/devices")?;
    let v: serde_json::Value = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
    let kbds = v["keyboards"].as_array().cloned().unwrap_or_default();
    match follow_index(&kbds, device, reference) {
        Some(index) => switch_layout("all", index),
        None => Ok(()),
    }
}

/// Read the current layout, and bring `watched` in line with it.
fn sync(watched: &str) -> std::io::Result<Snapshot> {
    let snap = snapshot(watched)?;
    if snap.watched_index.is_some_and(|i| i != snap.reference_index) {
        switch_layout(watched, snap.reference_index)?;
    }
    Ok(snap)
}

#[derive(Default)]
struct State {
    /// Bumped on every activelayout event of the watched (virtual) device.
    seq: u64,
    index: usize,
}

pub struct LayoutWatcher {
    state: Arc<(Mutex<State>, Condvar)>,
}

impl LayoutWatcher {
    /// `watched` is our virtual keyboard: it is kept on the same layout as the
    /// real keyboards, and `wait_event` waits for its layout events. Focus
    /// changes, closed windows, layout changes of the real keyboard and the
    /// socket closing are sent to `tx`.
    pub fn start(watched: &'static str, tx: Sender<Msg>) -> std::io::Result<Self> {
        let snap = sync(watched)?;
        let reference = snap.reference;
        let state = Arc::new((
            Mutex::new(State { index: snap.reference_index, ..Default::default() }),
            Condvar::new(),
        ));
        let stream = UnixStream::connect(socket_dir()?.join(".socket2.sock"))?;
        let st = state.clone();
        thread::spawn(move || {
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if let Some(rest) = line.strip_prefix("activewindow>>") {
                    let class = rest.split(',').next().unwrap_or("").to_string();
                    let _ = tx.send(Msg::FocusChanged(class));
                    continue;
                }
                // Comes right after activewindow (the class) for the same window.
                if let Some(addr) = line.strip_prefix("activewindowv2>>") {
                    let _ = tx.send(Msg::WindowFocused(addr.to_string()));
                    continue;
                }
                if let Some(addr) = line.strip_prefix("closewindow>>") {
                    let _ = tx.send(Msg::WindowClosed(addr.to_string()));
                    continue;
                }
                if line.starts_with("configreloaded>>") {
                    let _ = tx.send(Msg::HyprConfigReloaded);
                    continue;
                }
                if let Some(ns) = line.strip_prefix("openlayer>>") {
                    let _ = tx.send(Msg::Layer(ns.to_string(), true));
                    continue;
                }
                if let Some(ns) = line.strip_prefix("closelayer>>") {
                    let _ = tx.send(Msg::Layer(ns.to_string(), false));
                    continue;
                }
                let Some(rest) = line.strip_prefix("activelayout>>") else { continue };
                let device = rest.split(',').next().unwrap_or("");
                let (m, cv) = &*st;
                if device == watched {
                    m.lock().unwrap().seq += 1;
                    cv.notify_all();
                } else if device == reference {
                    // Layout names aren't indices; re-query to get the index.
                    match sync(watched) {
                        Ok(snap) => {
                            m.lock().unwrap().index = snap.reference_index;
                            let _ = tx.send(Msg::LayoutChanged(snap.reference_index));
                        }
                        Err(e) => eprintln!("layout sync failed: {e}"),
                    }
                } else if !device.contains("virtual") {
                    if let Err(e) = follow(device, &reference) {
                        eprintln!("layout follow failed: {e}");
                    }
                }
            }
            let _ = tx.send(Msg::HyprGone);
        });
        Ok(Self { state })
    }

    pub fn current_index(&self) -> usize {
        self.state.0.lock().unwrap().index
    }

    pub fn seq(&self) -> u64 {
        self.state.0.lock().unwrap().seq
    }

    /// Wait until a layout event for the watched device arrives after `since`.
    pub fn wait_event(&self, since: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (m, cv) = &*self.state;
        let mut s = m.lock().unwrap();
        loop {
            if s.seq > since {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            s = cv.wait_timeout(s, deadline - now).unwrap().0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::follow_index;
    use serde_json::json;

    fn kbds(cornix: u64, ite: u64) -> Vec<serde_json::Value> {
        vec![json!({"name": "ite-keyboard", "active_layout_index": ite}), json!({"name": "cornix", "active_layout_index": cornix})]
    }

    #[test]
    fn external_keyboard_switch_is_followed() {
        assert_eq!(follow_index(&kbds(1, 0), "cornix", "ite-keyboard"), Some(1));
        assert_eq!(follow_index(&kbds(0, 1), "cornix", "ite-keyboard"), Some(0));
    }

    #[test]
    fn nothing_to_follow() {
        assert_eq!(follow_index(&kbds(1, 1), "cornix", "ite-keyboard"), None);
        assert_eq!(follow_index(&kbds(1, 0), "gone", "ite-keyboard"), None);
    }
}
