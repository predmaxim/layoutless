//! The keyboard layout per window (`per_window = true` in the config).
//!
//! A window's layout is recorded when it changes while that window has focus,
//! and restored when the window is focused again. New windows start on the
//! first layout. Windows and shell popups listed in `first_layout_only` always
//! get the first layout and aren't remembered; when such a popup closes, the
//! layout from before it comes back.

use std::collections::HashMap;
use std::time::{Duration, Instant};

const FIRST: usize = 0;

/// Our own switch makes every keyboard announce the new layout. Those echoes
/// arrive within a few ms; ignoring them keeps a quick A -> B -> C focus run
/// from recording B's layout on C.
const ECHO_WINDOW: Duration = Duration::from_millis(50);

#[derive(Default)]
pub struct PerWindow {
    /// Window address as in Hyprland's events, without `0x`.
    focused: Option<String>,
    windows: HashMap<String, usize>,
    current: usize,
    switched_at: Option<Instant>,
    /// The focused window is on the first-layout-only list, so its layout isn't kept.
    first_only: bool,
    /// Layout to go back to when the first-layout-only popup closes.
    layer_saved: Option<usize>,
}

impl PerWindow {
    pub fn new(current: usize, focused: Option<String>) -> Self {
        Self { current, focused, ..Default::default() }
    }

    fn switch(&mut self, index: usize, now: Instant) -> Option<usize> {
        if index == self.current {
            return None;
        }
        self.current = index;
        self.switched_at = Some(now);
        Some(index)
    }

    /// Focus moved to `addr`; returns the layout to switch to, if any.
    pub fn focus(&mut self, addr: &str, first_only: bool, now: Instant) -> Option<usize> {
        if addr.is_empty() {
            self.focused = None;
            return None;
        }
        if self.focused.as_deref() == Some(addr) {
            return None;
        }
        self.focused = Some(addr.to_string());
        self.first_only = first_only;
        let wanted = if first_only { FIRST } else { *self.windows.get(addr).unwrap_or(&FIRST) };
        self.switch(wanted, now)
    }

    /// A first-layout-only popup (such as the Omarchy menu) took the keyboard.
    pub fn layer_open(&mut self, now: Instant) -> Option<usize> {
        self.layer_saved.get_or_insert(self.current);
        self.switch(FIRST, now)
    }

    /// The popup is gone; back to the layout from before it.
    pub fn layer_close(&mut self, now: Instant) -> Option<usize> {
        let saved = self.layer_saved.take()?;
        self.switch(saved, now)
    }

    /// The real keyboard reported layout `index`.
    pub fn layout_changed(&mut self, index: usize, now: Instant) {
        if self.switched_at.is_some_and(|at| now.duration_since(at) < ECHO_WINDOW) {
            return;
        }
        self.current = index;
        if self.first_only || self.layer_saved.is_some() {
            return;
        }
        if let Some(addr) = &self.focused {
            self.windows.insert(addr.clone(), index);
        }
    }

    pub fn close(&mut self, addr: &str) {
        self.windows.remove(addr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_layout_per_window() {
        let t0 = Instant::now();
        let later = |ms| t0 + Duration::from_millis(ms);
        let mut s = PerWindow::default();

        assert_eq!(s.focus("a", false, t0), None); // new window, already on us
        s.layout_changed(1, later(1000)); // user switches A to ru
        assert_eq!(s.focus("b", false, later(2000)), Some(0)); // new window B -> us
        s.layout_changed(1, later(2001)); // stale echo, ignored
        assert_eq!(s.windows.get("b"), None);
        assert_eq!(s.focus("a", false, later(3000)), Some(1)); // A back to ru
        assert_eq!(s.focus("", false, later(3500)), None); // empty workspace
        assert_eq!(s.focus("b", false, later(4000)), Some(0));
        s.close("a");
        assert_eq!(s.focus("a", false, later(5000)), None); // closed: treated as new, us
    }

    #[test]
    fn first_layout_only_windows_and_layers() {
        let t0 = Instant::now();
        let later = |ms| t0 + Duration::from_millis(ms);
        let mut s = PerWindow::default();

        s.focus("a", false, t0); // A on us
        assert_eq!(s.layer_open(later(1000)), None); // menu opens, already us
        s.layout_changed(1, later(1500)); // switched inside the menu: not A's
        assert_eq!(s.layer_close(later(2000)), Some(0)); // menu closes -> A's us
        assert_eq!(s.windows.get("a"), None);
        assert_eq!(s.layer_close(later(2100)), None); // stray close

        s.layout_changed(1, later(2500)); // A on ru
        assert_eq!(s.layer_open(later(3000)), Some(0)); // menu opens -> us
        assert_eq!(s.layer_close(later(3500)), Some(1)); // menu closes -> A's ru

        assert_eq!(s.focus("m", true, later(4000)), Some(0)); // listed window -> us
        s.layout_changed(1, later(5000));
        assert_eq!(s.windows.get("m"), None); // never remembered
        assert_eq!(s.focus("a", false, later(6000)), None); // A still ru, already on ru
        assert_eq!(s.focus("m", true, later(7000)), Some(0));
    }

    #[test]
    fn starts_with_the_focused_window() {
        let t0 = Instant::now();
        let mut s = PerWindow::new(1, Some("a".into()));
        s.layout_changed(1, t0); // the window focused at start is on ru
        assert_eq!(s.focus("b", false, t0 + Duration::from_secs(1)), Some(0));
        assert_eq!(s.focus("a", false, t0 + Duration::from_secs(2)), Some(1));
    }
}
