//! Lock state machine. Pure logic, no Win32: the hooks feed it input events
//! and act on its verdicts, so every rule here is unit-tested.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::config::Combo;
use crate::keys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Locked,
    Unlocked,
}

/// What a fully held combination will do once the hold completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Locked,
    Unlocked,
    Quit,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KeyVerdict {
    /// Drop the event so no app sees it.
    pub swallow: bool,
    /// Inject a no-op key first, so a passed-through Alt or Win press does not
    /// open a menu or the Start menu when its partner key was swallowed.
    pub mask: bool,
    /// A hold just started: the UI should begin animating.
    pub hold_started: bool,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    action: Action,
    since: Instant,
}

pub struct Engine {
    mode: Mode,
    toggle: Combo,
    quit: Option<Combo>,
    hold: Duration,
    /// Raw virtual keys currently down.
    held: BTreeSet<u16>,
    /// Keys whose first key-down was swallowed, so their repeats and key-up are too.
    swallowed_keys: BTreeSet<u16>,
    /// Bit per mouse button whose button-down was swallowed.
    swallowed_buttons: u8,
    pending: Option<Pending>,
    /// After a lock/unlock, ignore the combination until every key is released.
    await_release: bool,
}

impl Engine {
    pub fn new(start: Mode, toggle: Combo, quit: Option<Combo>, hold: Duration) -> Self {
        Self {
            mode: start,
            toggle,
            quit,
            hold,
            held: BTreeSet::new(),
            swallowed_keys: BTreeSet::new(),
            swallowed_buttons: 0,
            pending: None,
            await_release: false,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn on_key(&mut self, vk: u16, down: bool, now: Instant) -> KeyVerdict {
        let first_down = down && self.held.insert(vk);
        if !down {
            self.held.remove(&vk);
        }
        if self.held.is_empty() {
            self.await_release = false;
        }
        let was_pending = self.pending.is_some();
        self.update_pending(now);
        let hold_started = !was_pending && self.pending.is_some();

        if !down {
            let swallow = self.swallowed_keys.remove(&vk);
            return KeyVerdict {
                swallow,
                hold_started,
                ..Default::default()
            };
        }
        // A key whose first press was swallowed stays swallowed until released,
        // otherwise an app would see repeats or a key-up with no key-down.
        let swallow = self.swallowed_keys.contains(&vk)
            || self.mode == Mode::Locked
            || self.pending.is_some();
        if first_down && swallow {
            self.swallowed_keys.insert(vk);
        }
        let mask = swallow && self.mode == Mode::Unlocked && self.alt_or_win_held();
        KeyVerdict {
            swallow,
            mask,
            hold_started,
        }
    }

    /// `button` is 0..=4 (left, right, middle, X1, X2).
    pub fn on_button(&mut self, button: u8, down: bool) -> bool {
        let bit = 1u8 << button;
        if down {
            let swallow = self.mode == Mode::Locked;
            if swallow {
                self.swallowed_buttons |= bit;
            }
            swallow
        } else {
            let swallow = self.swallowed_buttons & bit != 0;
            self.swallowed_buttons &= !bit;
            swallow
        }
    }

    /// Mouse move and wheel events.
    pub fn on_motion(&self) -> bool {
        self.mode == Mode::Locked
    }

    /// Completes a hold that has lasted long enough.
    pub fn tick(&mut self, now: Instant) -> Option<Event> {
        let p = self.pending?;
        if now.duration_since(p.since) < self.hold {
            return None;
        }
        self.pending = None;
        self.await_release = true;
        Some(match (p.action, self.mode) {
            (Action::Quit, _) => Event::Quit,
            (Action::Toggle, Mode::Locked) => {
                self.mode = Mode::Unlocked;
                Event::Unlocked
            }
            (Action::Toggle, Mode::Unlocked) => {
                self.mode = Mode::Locked;
                Event::Locked
            }
        })
    }

    /// The hold in progress and how far along it is, from 0 to 1.
    pub fn progress(&self, now: Instant) -> Option<(Action, f32)> {
        let p = self.pending?;
        let t = now.duration_since(p.since).as_secs_f32() / self.hold.as_secs_f32();
        Some((p.action, t.min(1.0)))
    }

    /// Forgets every held key. Called when the secure desktop (Ctrl+Alt+Del,
    /// UAC) takes over, because key-ups that happen there never reach the hook.
    pub fn reset_keys(&mut self) {
        self.held.clear();
        self.swallowed_keys.clear();
        self.swallowed_buttons = 0;
        self.pending = None;
        self.await_release = false;
    }

    fn update_pending(&mut self, now: Instant) {
        let action = self.matching_action();
        self.pending = match (self.pending, action) {
            (Some(p), Some(a)) if p.action == a => Some(p),
            (_, Some(action)) => Some(Pending { action, since: now }),
            (_, None) => None,
        };
    }

    /// The action whose combination is held exactly (no extra keys).
    fn matching_action(&self) -> Option<Action> {
        if self.await_release || self.held.is_empty() {
            return None;
        }
        let held: Combo = self.held.iter().map(|&vk| keys::normalize(vk)).collect();
        if held == self.toggle {
            Some(Action::Toggle)
        } else if self.mode == Mode::Unlocked && self.quit.as_ref() == Some(&held) {
            Some(Action::Quit)
        } else {
            None
        }
    }

    fn alt_or_win_held(&self) -> bool {
        self.held
            .iter()
            .any(|&vk| matches!(keys::normalize(vk), keys::VK_MENU | keys::VK_LWIN))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LCTRL: u16 = 0xA2;
    const LALT: u16 = 0xA4;
    const LSHIFT: u16 = 0xA0;
    const U: u16 = 0x55;
    const Q: u16 = 0x51;
    const A: u16 = 0x41;
    const HOLD: Duration = Duration::from_millis(1000);

    fn engine(mode: Mode) -> Engine {
        let toggle = Combo::from([keys::VK_CONTROL, keys::VK_MENU, U]);
        let quit = Combo::from([keys::VK_CONTROL, keys::VK_MENU, keys::VK_SHIFT, Q]);
        Engine::new(mode, toggle, Some(quit), HOLD)
    }

    fn press(e: &mut Engine, keys: &[u16], t: Instant) -> Vec<KeyVerdict> {
        keys.iter().map(|&k| e.on_key(k, true, t)).collect()
    }

    fn release(e: &mut Engine, keys: &[u16], t: Instant) -> Vec<KeyVerdict> {
        keys.iter().map(|&k| e.on_key(k, false, t)).collect()
    }

    #[test]
    fn locked_swallows_everything() {
        let mut e = engine(Mode::Locked);
        let t = Instant::now();
        assert!(e.on_key(A, true, t).swallow);
        assert!(e.on_key(A, false, t).swallow);
        assert!(e.on_button(0, true));
        assert!(e.on_button(0, false));
        assert!(e.on_motion());
    }

    #[test]
    fn unlocked_passes_normal_input() {
        let mut e = engine(Mode::Unlocked);
        let t = Instant::now();
        assert_eq!(e.on_key(A, true, t), KeyVerdict::default());
        assert!(!e.on_key(A, false, t).swallow);
        assert!(!e.on_button(1, true));
        assert!(!e.on_motion());
    }

    #[test]
    fn holding_toggle_unlocks_only_after_hold_time() {
        let mut e = engine(Mode::Locked);
        let t = Instant::now();
        let v = press(&mut e, &[LCTRL, LALT, U], t);
        assert!(v.iter().all(|v| v.swallow));
        assert!(v[2].hold_started);
        assert_eq!(e.tick(t + HOLD / 2), None);
        assert_eq!(e.progress(t + HOLD / 2).map(|p| p.0), Some(Action::Toggle));
        assert_eq!(e.tick(t + HOLD), Some(Event::Unlocked));
        assert_eq!(e.mode(), Mode::Unlocked);
        // Keys pressed while locked stay swallowed through their release.
        assert!(e.on_key(U, true, t + HOLD).swallow, "repeat after unlock");
        assert!(
            release(&mut e, &[U, LALT, LCTRL], t + HOLD)
                .iter()
                .all(|v| v.swallow)
        );
        // And input now flows.
        assert!(!e.on_key(A, true, t + HOLD).swallow);
    }

    #[test]
    fn releasing_early_cancels() {
        let mut e = engine(Mode::Locked);
        let t = Instant::now();
        press(&mut e, &[LCTRL, LALT, U], t);
        release(&mut e, &[U], t + HOLD / 2);
        assert_eq!(e.progress(t + HOLD), None);
        assert_eq!(e.tick(t + HOLD * 2), None);
        assert_eq!(e.mode(), Mode::Locked);
    }

    #[test]
    fn extra_key_blocks_combo() {
        let mut e = engine(Mode::Locked);
        let t = Instant::now();
        press(&mut e, &[LCTRL, LALT, A, U], t);
        assert_eq!(e.tick(t + HOLD * 2), None);
    }

    #[test]
    fn right_side_modifiers_count() {
        let mut e = engine(Mode::Locked);
        let t = Instant::now();
        press(&mut e, &[0xA3, 0xA5, U], t);
        assert_eq!(e.tick(t + HOLD), Some(Event::Unlocked));
    }

    #[test]
    fn needs_full_release_before_toggling_again() {
        let mut e = engine(Mode::Locked);
        let t = Instant::now();
        press(&mut e, &[LCTRL, LALT, U], t);
        assert_eq!(e.tick(t + HOLD), Some(Event::Unlocked));
        // Still holding: must not flip back.
        press(&mut e, &[U], t + HOLD);
        assert_eq!(e.tick(t + HOLD * 3), None);
        release(&mut e, &[U], t + HOLD * 3);
        press(&mut e, &[U], t + HOLD * 3);
        assert_eq!(e.tick(t + HOLD * 5), None, "partial release is not enough");
        release(&mut e, &[U, LALT, LCTRL], t + HOLD * 5);
        let t2 = t + HOLD * 6;
        press(&mut e, &[LCTRL, LALT, U], t2);
        assert_eq!(e.tick(t2 + HOLD), Some(Event::Locked));
    }

    #[test]
    fn locking_from_unlocked_hides_completing_key_and_masks_alt() {
        let mut e = engine(Mode::Unlocked);
        let t = Instant::now();
        let v = press(&mut e, &[LCTRL, LALT, U], t);
        assert!(
            !v[0].swallow && !v[1].swallow,
            "modifiers pass until the combo is complete"
        );
        assert!(v[2].swallow && v[2].mask);
        assert_eq!(e.tick(t + HOLD), Some(Event::Locked));
        let up = release(&mut e, &[U, LALT, LCTRL], t + HOLD);
        assert!(up[0].swallow, "U down was swallowed, so its up is too");
        assert!(
            !up[1].swallow && !up[2].swallow,
            "modifier ups pass so apps see them released"
        );
        assert!(e.on_key(A, true, t + HOLD).swallow);
    }

    #[test]
    fn modifier_repeat_during_hold_is_swallowed_but_release_passes() {
        let mut e = engine(Mode::Unlocked);
        let t = Instant::now();
        press(&mut e, &[LCTRL, LALT, U], t);
        assert!(e.on_key(LCTRL, true, t).swallow, "repeat while combo held");
        release(&mut e, &[U], t);
        assert!(!e.on_key(LCTRL, false, t).swallow);
    }

    #[test]
    fn quit_only_when_unlocked() {
        let mut locked = engine(Mode::Locked);
        let t = Instant::now();
        press(&mut locked, &[LCTRL, LALT, LSHIFT, Q], t);
        assert_eq!(locked.tick(t + HOLD), None);

        let mut e = engine(Mode::Unlocked);
        press(&mut e, &[LCTRL, LALT, LSHIFT, Q], t);
        assert_eq!(e.progress(t).map(|p| p.0), Some(Action::Quit));
        assert_eq!(e.tick(t + HOLD), Some(Event::Quit));
    }

    #[test]
    fn mouse_button_pressed_before_lock_releases_normally() {
        let mut e = engine(Mode::Unlocked);
        let t = Instant::now();
        assert!(!e.on_button(0, true));
        press(&mut e, &[LCTRL, LALT, U], t);
        e.tick(t + HOLD);
        assert!(
            !e.on_button(0, false),
            "up must reach the app or the drag sticks"
        );
        assert!(e.on_button(0, true));
        assert!(e.on_button(0, false));
    }

    #[test]
    fn reset_clears_stuck_keys() {
        let mut e = engine(Mode::Locked);
        let t = Instant::now();
        press(&mut e, &[LCTRL, LALT], t);
        e.reset_keys();
        press(&mut e, &[U], t);
        assert_eq!(
            e.tick(t + HOLD),
            None,
            "Ctrl+Alt were lost on the secure desktop"
        );
        press(&mut e, &[LCTRL, LALT], t);
        e.on_key(U, false, t);
        press(&mut e, &[U], t);
        assert_eq!(e.tick(t + HOLD), Some(Event::Unlocked));
    }
}
