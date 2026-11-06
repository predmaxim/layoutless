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
    /// Layout index of the keyboard the current layout is read from.
    reference_index: usize,
    watched_index: Option<usize>,
}

/// Whether someone can type on a keyboard Hyprland lists. Left out, as Omarchy's
/// layout widget leaves them out (UNTYPED_KEYBOARDS in
/// shell/plugins/bar/widgets/KeyboardLayoutModel.js): virtual keyboards (fcitx5's,
/// wtype's, our own), ACPI buttons and hotkey blocks, which carry the layout list
/// without being typed on.
fn is_typed(name: &str) -> bool {
    const UNTYPED_PREFIXES: [&str; 4] = ["power-button", "sleep-button", "lid-switch", "video-bus"];
    const UNTYPED_SUFFIXES: [&str; 2] = ["-wmi-hotkeys", "-wireless-radio-control"];
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let base = if base.len() < name.len() { base.strip_suffix('-').unwrap_or(name) } else { name };
    !(name.contains("virtual")
        || UNTYPED_PREFIXES.iter().any(|p| name.starts_with(p))
        || UNTYPED_SUFFIXES.iter().any(|s| base.ends_with(s)))
}

/// The keyboard the current layout is read from, chosen as Omarchy's layout
/// widget chooses it (selectKeyboard): the one the last layout switch named, as
/// every keyboard keeps a layout of its own and Alt+Alt switches only the one it
/// is typed on; before any switch, the furthest along its layout list.
///
/// Never the keyboard flagged `main`: with fcitx5 running, `main` is its virtual
/// keyboard, which `switchxkblayout all` doesn't touch.
fn pick_reference<'a>(kbds: &'a [serde_json::Value], switched: &str) -> Option<&'a serde_json::Value> {
    let index = |k: &serde_json::Value| k["active_layout_index"].as_u64().unwrap_or(0);
    let mut typed = kbds.iter().filter(|k| is_typed(k["name"].as_str().unwrap_or("")));
    let named = kbds.iter().find(|k| !switched.is_empty() && k["name"] == switched && is_typed(switched));
    named.or_else(|| typed.next().map(|first| typed.fold(first, |far, k| if index(k) > index(far) { k } else { far })))
}

/// Read layout indices from `hyprctl devices`.
fn snapshot(watched: &str, switched: &str) -> std::io::Result<Snapshot> {
    let reply = command("j/devices")?;
    let v: serde_json::Value = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
    let kbds = v["keyboards"].as_array().cloned().unwrap_or_default();
    let index = |k: &serde_json::Value| k["active_layout_index"].as_u64().unwrap_or(0) as usize;
    let reference = pick_reference(&kbds, switched).ok_or_else(|| std::io::Error::other("no real keyboard found"))?;
    Ok(Snapshot {
        reference_index: index(reference),
        watched_index: kbds.iter().find(|k| k["name"] == watched).map(index),
    })
}

/// Read the current layout, and bring `watched` in line with it.
fn sync(watched: &str, switched: &str) -> std::io::Result<Snapshot> {
    let snap = snapshot(watched, switched)?;
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
    /// keyboard being typed on, and `wait_event` waits for its layout events.
    /// Focus changes, closed windows, layout changes of a real keyboard and the
    /// socket closing are sent to `tx`.
    pub fn start(watched: &'static str, tx: Sender<Msg>) -> std::io::Result<Self> {
        let snap = sync(watched, "")?;
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
                } else if is_typed(device) {
                    // Layout names aren't indices; re-query to get the index.
                    match sync(watched, device) {
                        Ok(snap) => {
                            m.lock().unwrap().index = snap.reference_index;
                            let _ = tx.send(Msg::LayoutChanged(snap.reference_index));
                        }
                        Err(e) => eprintln!("layout sync failed: {e}"),
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
    use super::{is_typed, pick_reference};
    use serde_json::json;

    fn kbds(cornix: u64) -> Vec<serde_json::Value> {
        vec![
            json!({"name": "video-bus", "active_layout_index": 0}),
            json!({"name": "hl-virtual-keyboard-fcitx5", "active_layout_index": 1}),
            json!({"name": "ite-tech.-inc.-ite-device(8176)-keyboard", "active_layout_index": 0}),
            json!({"name": "cornix", "active_layout_index": cornix}),
        ]
    }

    #[test]
    fn the_keyboard_switched_last_is_read() {
        assert_eq!(pick_reference(&kbds(0), "cornix").unwrap()["name"], "cornix");
    }

    #[test]
    fn before_a_switch_the_furthest_along_is_read() {
        assert_eq!(pick_reference(&kbds(1), "").unwrap()["name"], "cornix");
        assert_eq!(pick_reference(&kbds(0), "").unwrap()["name"], "ite-tech.-inc.-ite-device(8176)-keyboard");
        assert_eq!(pick_reference(&kbds(1), "unplugged").unwrap()["name"], "cornix");
    }

    #[test]
    fn untyped_keyboards_are_never_read() {
        assert_eq!(pick_reference(&kbds(0), "hl-virtual-keyboard-fcitx5").unwrap()["name"], "ite-tech.-inc.-ite-device(8176)-keyboard");
        for name in ["power-button-1", "video-bus", "layoutless-virtual-keyboard", "ite-tech.-inc.-ite-device(8176)-wireless-radio-control", "dell-wmi-hotkeys-2"] {
            assert!(!is_typed(name), "{name}");
        }
        for name in ["cornix", "at-translated-set-2-keyboard", "cornix-consumer-control"] {
            assert!(is_typed(name), "{name}");
        }
    }
}
