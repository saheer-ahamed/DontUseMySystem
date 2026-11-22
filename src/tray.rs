//! Notification-area icon. It shows the lock state, and while unlocked its menu
//! offers Lock now and Quit. While locked the menu never opens: input is
//! blocked anyway, and the tray must never become a way around the lock.

use std::sync::OnceLock;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, ChangeWindowMessageFilterEx, CreatePopupMenu, DestroyIcon, DestroyMenu, HICON,
    IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSGFLT_ALLOW,
    PostMessageW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_APP, WM_CONTEXTMENU, WM_NULL, WM_USER,
};
use windows::core::{PCSTR, PCWSTR, w};

use crate::log;

/// Callback message the shell sends for clicks on the icon.
pub const WM_TRAY: u32 = WM_APP + 2;
pub const CMD_LOCK: u32 = 1;
pub const CMD_QUIT: u32 = 2;

const ICON_ID: u32 = 1;
/// Resource ids from assets/app.rc.
const RES_LOCKED: u16 = 2;
const RES_UNLOCKED: u16 = 3;
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;

/// Broadcast when Explorer (re)starts. A new taskbar has forgotten every icon.
pub fn taskbar_created() -> u32 {
    static MSG: OnceLock<u32> = OnceLock::new();
    *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

pub struct Tray {
    hwnd: HWND,
    instance: HINSTANCE,
    locked_icon: HICON,
    unlocked_icon: HICON,
    locked: bool,
    toggle: String,
}

impl Tray {
    pub fn new(hwnd: HWND, instance: HINSTANCE, locked: bool, toggle: String) -> Self {
        // When elevated, UIPI drops messages from the non-elevated Explorer
        // unless they are allowed explicitly.
        for msg in [taskbar_created(), WM_TRAY] {
            unsafe {
                let _ = ChangeWindowMessageFilterEx(hwnd, msg, MSGFLT_ALLOW, None);
            }
        }
        follow_dark_mode();
        let (locked_icon, unlocked_icon) = load_icons(instance);
        let tray = Self {
            hwnd,
            instance,
            locked_icon,
            unlocked_icon,
            locked,
            toggle,
        };
        tray.add();
        tray
    }

    /// Also called on `TaskbarCreated`, to put the icon back after Explorer restarts.
    pub fn add(&self) {
        let mut d = self.data();
        unsafe {
            if !Shell_NotifyIconW(NIM_ADD, &d).as_bool() {
                log::line("tray icon add failed");
                return;
            }
            d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
        }
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        self.modify();
    }

    /// Reloads the icons at the primary monitor's current DPI.
    pub fn reload_icons(&mut self) {
        self.destroy_icons();
        (self.locked_icon, self.unlocked_icon) = load_icons(self.instance);
        self.modify();
    }

    fn modify(&self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data());
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut d = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: if self.locked {
                self.locked_icon
            } else {
                self.unlocked_icon
            },
            ..Default::default()
        };
        let tip = tooltip(self.locked, &self.toggle);
        let len = d.szTip.len() - 1;
        for (dst, src) in d.szTip.iter_mut().zip(tip.encode_utf16().take(len)) {
            *dst = src;
        }
        d
    }

    fn destroy_icons(&self) {
        for icon in [self.locked_icon, self.unlocked_icon] {
            if !icon.is_invalid() {
                unsafe {
                    let _ = DestroyIcon(icon);
                }
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
        }
        self.destroy_icons();
    }
}

/// Lets popup menus follow the system dark mode, as Explorer's own menus do.
/// Win32 has no documented switch for this; uxtheme's `SetPreferredAppMode`
/// (ordinal 135, Windows 10 1903+) is what Notepad++, Firefox and others use.
/// When it is missing, menus simply stay light.
fn follow_dark_mode() {
    const ALLOW_DARK: i32 = 1;
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        if let Some(f) = GetProcAddress(uxtheme, PCSTR(135 as *const u8)) {
            let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 =
                std::mem::transmute(f);
            set_preferred_app_mode(ALLOW_DARK);
        }
    }
}

fn tooltip(locked: bool, toggle: &str) -> String {
    let (state, verb) = if locked {
        ("locked", "unlock")
    } else {
        ("unlocked", "lock")
    };
    format!("DontUseMySystem: {state}\nHold {toggle} to {verb}")
}

fn load_icons(instance: HINSTANCE) -> (HICON, HICON) {
    let size = unsafe {
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let (mut dpi, mut unused) = (96, 96);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
        GetSystemMetricsForDpi(SM_CXSMICON, dpi)
    };
    let load = |id: u16| unsafe {
        LoadImageW(
            Some(instance),
            PCWSTR(id as usize as *const u16),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
        .map(|h| HICON(h.0))
        .unwrap_or_else(|e| {
            log::line(&format!("tray icon {id} load failed: {e}"));
            HICON::default()
        })
    };
    (load(RES_LOCKED), load(RES_UNLOCKED))
}

/// The anchor point when a tray callback asks for the menu: a left click,
/// a right click, or Enter / Space on the focused icon.
pub fn menu_request(wp: WPARAM, lp: LPARAM) -> Option<POINT> {
    let event = (lp.0 & 0xFFFF) as u32;
    matches!(event, WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT).then(|| POINT {
        x: (wp.0 & 0xFFFF) as u16 as i16 as i32,
        y: ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
    })
}

/// Shows the menu and returns the chosen command, or 0. Must not be called
/// while the App is borrowed: the menu's modal loop keeps dispatching our
/// timer messages, which borrow it.
pub fn show_menu(hwnd: HWND, at: POINT, toggle: &str) -> u32 {
    let header: Vec<u16> = format!("Unlocked - hold {toggle} to lock")
        .encode_utf16()
        .chain([0])
        .collect();
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(header.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_LOCK as usize, w!("&Lock now"));
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT as usize, w!("&Quit"));
        // Without this the menu stays open when you click elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY).0,
            at.x,
            at.y,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0 as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_names_state_and_next_action() {
        assert_eq!(
            tooltip(true, "Ctrl+Alt+U"),
            "DontUseMySystem: locked\nHold Ctrl+Alt+U to unlock"
        );
        assert_eq!(
            tooltip(false, "Ctrl+Alt+U"),
            "DontUseMySystem: unlocked\nHold Ctrl+Alt+U to lock"
        );
    }

    #[test]
    fn menu_opens_on_clicks_and_keyboard_only() {
        let wp = WPARAM((200usize << 16) | 1500);
        for event in [WM_CONTEXTMENU, NIN_SELECT, NIN_KEYSELECT] {
            let at = menu_request(wp, LPARAM(event as isize | (1 << 16))).unwrap();
            assert_eq!((at.x, at.y), (1500, 200));
        }
        // Mouse moves and hover popups do not open it.
        assert!(menu_request(wp, LPARAM(0x200)).is_none());
        // Negative coordinates on a monitor left of the primary one.
        let at = menu_request(WPARAM(0xFF9C), LPARAM(WM_CONTEXTMENU as isize)).unwrap();
        assert_eq!(at.x, -100);
    }
}
