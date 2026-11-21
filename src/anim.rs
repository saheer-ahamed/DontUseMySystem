//! Glow animation timeline. Pure, so it is unit-tested apart from rendering.

use std::time::{Duration, Instant};

use crate::config::Rgb;

const FLASH_FADE: Duration = Duration::from_millis(650);
const CANCEL_FADE: Duration = Duration::from_millis(180);
/// Glow level the moment a hold starts, so feedback is instant.
const HOLD_FLOOR: f32 = 0.18;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub color: Rgb,
    /// 0 to 1.
    pub intensity: f32,
}

#[derive(Debug, Clone, Copy)]
enum State {
    Idle,
    Hold {
        color: Rgb,
        intensity: f32,
    },
    Fade {
        color: Rgb,
        from: f32,
        start: Instant,
        dur: Duration,
    },
}

pub struct Anim {
    state: State,
}

impl Anim {
    pub fn new() -> Self {
        Self { state: State::Idle }
    }

    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        matches!(self.state, State::Idle)
    }

    /// Full-strength flash that fades out: confirms a lock or unlock.
    pub fn flash(&mut self, color: Rgb, now: Instant) {
        self.state = State::Fade {
            color,
            from: 1.0,
            start: now,
            dur: FLASH_FADE,
        };
    }

    /// Advances one frame. `hold` is the color and progress of a hold in
    /// progress, if any. Returns `None` once the glow is fully gone.
    pub fn step(&mut self, now: Instant, hold: Option<(Rgb, f32)>) -> Option<Frame> {
        if let Some((color, progress)) = hold {
            let intensity = HOLD_FLOOR + (1.0 - HOLD_FLOOR) * smoothstep(progress);
            self.state = State::Hold { color, intensity };
            return Some(Frame { color, intensity });
        }
        if let State::Hold { color, intensity } = self.state {
            self.state = State::Fade {
                color,
                from: intensity,
                start: now,
                dur: CANCEL_FADE,
            };
        }
        match self.state {
            State::Idle | State::Hold { .. } => {
                self.state = State::Idle;
                None
            }
            State::Fade {
                color,
                from,
                start,
                dur,
            } => {
                let t = now.duration_since(start).as_secs_f32() / dur.as_secs_f32();
                if t >= 1.0 {
                    self.state = State::Idle;
                    return None;
                }
                let eased = 1.0 - (1.0 - t).powi(3);
                Some(Frame {
                    color,
                    intensity: from * (1.0 - eased),
                })
            }
        }
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: Rgb = Rgb(1, 2, 3);

    #[test]
    fn hold_ramps_up() {
        let mut a = Anim::new();
        let t = Instant::now();
        let start = a.step(t, Some((C, 0.0))).unwrap().intensity;
        let mid = a.step(t, Some((C, 0.5))).unwrap().intensity;
        let end = a.step(t, Some((C, 1.0))).unwrap().intensity;
        assert!(start > 0.0 && start < mid && mid < end);
        assert!((end - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cancel_fades_from_current_level() {
        let mut a = Anim::new();
        let t = Instant::now();
        let held = a.step(t, Some((C, 0.5))).unwrap().intensity;
        let first = a.step(t, None).unwrap().intensity;
        assert!((first - held).abs() < 1e-6);
        assert!(a.step(t + CANCEL_FADE / 2, None).unwrap().intensity < held);
        assert_eq!(a.step(t + CANCEL_FADE, None), None);
        assert!(a.is_idle());
    }

    #[test]
    fn flash_fades_to_idle() {
        let mut a = Anim::new();
        let t = Instant::now();
        a.flash(C, t);
        assert_eq!(a.step(t, None).unwrap().intensity, 1.0);
        assert_eq!(a.step(t + FLASH_FADE, None), None);
        assert!(a.is_idle());
    }
}
