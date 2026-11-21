//! User config, stored as TOML in `%APPDATA%\DontUseMySystem\config.toml`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::keys;

pub const DEFAULT_TOML: &str = r##"# DontUseMySystem config. Restart the app after editing.

# Hold these keys together to unlock. Hold them again to lock.
toggle_keys = ["Ctrl", "Alt", "U"]

# Hold these keys together, while unlocked, to quit. Delete the line to disable.
quit_keys = ["Ctrl", "Alt", "Shift", "Q"]

# How long a combination must be held, in milliseconds.
hold_ms = 1200

# Start locked when the app launches.
start_locked = true

# Switch the touchpad off while locked. Three- and four-finger gestures bypass
# input hooks, so this is the only way to block them. It is switched back on
# when you unlock or quit.
disable_touchpad = true

# Glow colors (#RRGGBB) and glow width in pixels at 100% display scaling.
unlock_color = "#00E5FF"
lock_color = "#FF3D7F"
quit_color = "#FFB020"
glow_width = 36
"##;

/// A key combination, stored as normalized virtual keys.
pub type Combo = BTreeSet<u16>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Debug, Clone)]
pub struct Config {
    pub toggle_keys: Combo,
    pub quit_keys: Option<Combo>,
    pub hold: Duration,
    pub start_locked: bool,
    pub disable_touchpad: bool,
    pub unlock_color: Rgb,
    pub lock_color: Rgb,
    pub quit_color: Rgb,
    pub glow_width: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    toggle_keys: Vec<String>,
    quit_keys: Option<Vec<String>>,
    hold_ms: u64,
    start_locked: bool,
    /// Optional so config files written before this setting existed still load.
    #[serde(default = "enabled")]
    disable_touchpad: bool,
    unlock_color: String,
    lock_color: String,
    quit_color: String,
    glow_width: u32,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: Raw = toml::from_str(text).map_err(|e| e.to_string())?;
        let toggle_keys = combo(&raw.toggle_keys, "toggle_keys")?;
        let quit_keys = raw.quit_keys.map(|k| combo(&k, "quit_keys")).transpose()?;
        if quit_keys.as_ref() == Some(&toggle_keys) {
            return Err("quit_keys must differ from toggle_keys".into());
        }
        if !(200..=10_000).contains(&raw.hold_ms) {
            return Err("hold_ms must be between 200 and 10000".into());
        }
        if !(4..=200).contains(&raw.glow_width) {
            return Err("glow_width must be between 4 and 200".into());
        }
        Ok(Self {
            toggle_keys,
            quit_keys,
            hold: Duration::from_millis(raw.hold_ms),
            start_locked: raw.start_locked,
            disable_touchpad: raw.disable_touchpad,
            unlock_color: color(&raw.unlock_color, "unlock_color")?,
            lock_color: color(&raw.lock_color, "lock_color")?,
            quit_color: color(&raw.quit_color, "quit_color")?,
            glow_width: raw.glow_width,
        })
    }

    /// Reads the config, writing the default file first if none exists.
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(path, DEFAULT_TOML).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
        base.join("DontUseMySystem").join("config.toml")
    }
}

fn enabled() -> bool {
    true
}

fn combo(names: &[String], field: &str) -> Result<Combo, String> {
    if names.is_empty() {
        return Err(format!("{field} must list at least one key"));
    }
    names
        .iter()
        .map(|n| keys::parse(n).ok_or_else(|| format!("{field}: unknown key \"{n}\"")))
        .collect()
}

fn color(s: &str, field: &str) -> Result<Rgb, String> {
    let hex = s.strip_prefix('#').filter(|h| h.len() == 6 && h.is_ascii());
    let parsed = hex.and_then(|h| {
        let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        Some(Rgb(c(0)?, c(2)?, c(4)?))
    });
    parsed.ok_or_else(|| format!("{field} must look like \"#RRGGBB\", got \"{s}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses() {
        let c = Config::parse(DEFAULT_TOML).unwrap();
        assert_eq!(
            c.toggle_keys,
            Combo::from([keys::VK_CONTROL, keys::VK_MENU, 0x55])
        );
        assert_eq!(c.hold, Duration::from_millis(1200));
        assert_eq!(c.unlock_color, Rgb(0x00, 0xE5, 0xFF));
        assert!(c.quit_keys.is_some());
    }

    #[test]
    fn disable_touchpad_defaults_on_for_older_files() {
        assert!(Config::parse(DEFAULT_TOML).unwrap().disable_touchpad);
        let old = DEFAULT_TOML.replace("disable_touchpad = true", "");
        assert!(Config::parse(&old).unwrap().disable_touchpad);
        let off = DEFAULT_TOML.replace("disable_touchpad = true", "disable_touchpad = false");
        assert!(!Config::parse(&off).unwrap().disable_touchpad);
    }

    #[test]
    fn quit_keys_optional() {
        let text = DEFAULT_TOML.replace("quit_keys = [\"Ctrl\", \"Alt\", \"Shift\", \"Q\"]", "");
        assert!(Config::parse(&text).unwrap().quit_keys.is_none());
    }

    #[test]
    fn rejects_bad_values() {
        let bad =
            |from: &str, to: &str| Config::parse(&DEFAULT_TOML.replace(from, to)).unwrap_err();
        assert!(bad("\"U\"]", "\"Nope\"]").contains("unknown key"));
        assert!(bad("#00E5FF", "00E5FF").contains("#RRGGBB"));
        assert!(bad("#00E5FF", "#00E5FG").contains("#RRGGBB"));
        assert!(bad("hold_ms = 1200", "hold_ms = 5").contains("hold_ms"));
        assert!(bad("\"Shift\", \"Q\"", "\"U\"").contains("differ"));
        assert!(bad("glow_width = 36", "glow_width = 36\nextra = 1").contains("extra"));
    }
}
