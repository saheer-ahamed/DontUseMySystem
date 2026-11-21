//! Virtual-key names and left/right normalization.

pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;
const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LMENU: u16 = 0xA4;
const VK_RMENU: u16 = 0xA5;

/// Folds left/right modifier variants into one key, so "Ctrl" matches either Ctrl key.
pub fn normalize(vk: u16) -> u16 {
    match vk {
        VK_LSHIFT | VK_RSHIFT => VK_SHIFT,
        VK_LCONTROL | VK_RCONTROL => VK_CONTROL,
        VK_LMENU | VK_RMENU => VK_MENU,
        VK_RWIN => VK_LWIN,
        other => other,
    }
}

/// Parses a key name from the config file (case-insensitive).
pub fn parse(name: &str) -> Option<u16> {
    let n = name.trim().to_ascii_lowercase();
    let vk = match n.as_str() {
        "ctrl" | "control" => VK_CONTROL,
        "alt" => VK_MENU,
        "shift" => VK_SHIFT,
        "win" | "windows" | "super" => VK_LWIN,
        "space" => 0x20,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "backspace" => 0x08,
        "insert" => 0x2D,
        "delete" | "del" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "pause" => 0x13,
        "scrolllock" => 0x91,
        _ => return parse_char_or_fn(&n),
    };
    Some(vk)
}

fn parse_char_or_fn(n: &str) -> Option<u16> {
    let bytes = n.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
        return Some(u16::from(bytes[0].to_ascii_uppercase()));
    }
    let num: u16 = n.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&num).then(|| 0x70 + num - 1)
}

/// Human-readable name for logs.
pub fn name(vk: u16) -> String {
    match normalize(vk) {
        VK_CONTROL => "Ctrl".into(),
        VK_MENU => "Alt".into(),
        VK_SHIFT => "Shift".into(),
        VK_LWIN => "Win".into(),
        v @ (0x30..=0x39 | 0x41..=0x5A) => char::from(v as u8).to_string(),
        v @ 0x70..=0x87 => format!("F{}", v - 0x70 + 1),
        v => format!("0x{v:02X}"),
    }
}

/// "Ctrl+Alt+Shift+Win+X" order, the way Windows writes shortcuts.
pub fn combo_name(keys: &std::collections::BTreeSet<u16>) -> String {
    let rank = |vk: u16| match vk {
        VK_CONTROL => 0,
        VK_MENU => 1,
        VK_SHIFT => 2,
        VK_LWIN => 3,
        _ => 4,
    };
    let mut sorted: Vec<u16> = keys.iter().copied().collect();
    sorted.sort_by_key(|&vk| (rank(vk), vk));
    sorted.into_iter().map(name).collect::<Vec<_>>().join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_names_in_windows_order() {
        let set = [0x51, VK_SHIFT, VK_MENU, VK_CONTROL].into_iter().collect();
        assert_eq!(combo_name(&set), "Ctrl+Alt+Shift+Q");
    }

    #[test]
    fn parses_names() {
        assert_eq!(parse("Ctrl"), Some(VK_CONTROL));
        assert_eq!(parse("ALT"), Some(VK_MENU));
        assert_eq!(parse("u"), Some(0x55));
        assert_eq!(parse("7"), Some(0x37));
        assert_eq!(parse("F1"), Some(0x70));
        assert_eq!(parse("f24"), Some(0x87));
        assert_eq!(parse("F25"), None);
        assert_eq!(parse("F0"), None);
        assert_eq!(parse("nope"), None);
    }

    #[test]
    fn folds_left_right() {
        assert_eq!(normalize(VK_LCONTROL), VK_CONTROL);
        assert_eq!(normalize(VK_RMENU), VK_MENU);
        assert_eq!(normalize(VK_RWIN), VK_LWIN);
        assert_eq!(normalize(0x55), 0x55);
    }

    #[test]
    fn names_round_trip() {
        for n in ["Ctrl", "Alt", "Shift", "Win", "U", "7", "F12"] {
            assert_eq!(name(parse(n).unwrap()), n);
        }
    }
}
