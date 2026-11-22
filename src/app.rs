//! Win32 side: low-level input hooks, the hidden controller window that owns
//! timers, and the message loop. Everything runs on one thread: Windows calls
//! low-level hooks on the thread that installed them, from its message loop.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, HIGH_PRIORITY_CLASS, SetPriorityClass};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, EVENT_SYSTEM_DESKTOPSWITCH,
    EndMenu, GetMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, KillTimer, MSG, MSLLHOOKSTRUCT,
    PostMessageW, PostQuitMessage, RegisterClassExW, SetTimer, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT, WM_APP,
    WM_DISPLAYCHANGE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_TIMER, WM_XBUTTONDOWN, WM_XBUTTONUP,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use crate::anim::Anim;
use crate::config::{Config, Rgb};
use crate::engine::{Action, Engine, Event, Mode};
use crate::overlay::{self, Overlay};
use crate::touchpad::Touchpad;
use crate::tray::{self, Tray};
use crate::{keys, log};

/// Marks the no-op key we inject, so our own hook lets it through.
const MASK_TAG: usize = 0x4455_4D53; // "DUMS"
/// Unassigned virtual key, the same trick AutoHotkey uses to mask Alt/Win.
const MASK_VK: u16 = 0xE8;

const WM_APP_WAKE: u32 = WM_APP + 1;
const TIMER_ANIM: usize = 1;
const TIMER_REARM: usize = 2;
const TIMER_FAILSAFE: usize = 3;
const FRAME_MS: u32 = 10;
/// Windows silently drops a low-level hook whose callback ever runs past
/// `LowLevelHooksTimeout`, with no notification. Re-installing on a timer
/// bounds how long input could leak if that ever happens.
const REARM_MS: u32 = 3000;

pub struct Options {
    pub failsafe: Option<Duration>,
}

struct App {
    controller: HWND,
    overlay: Overlay,
    anim: Anim,
    animating: bool,
    was_holding: bool,
    hooks: Hooks,
    /// `None` when `disable_touchpad` is off.
    touchpad: Option<Touchpad>,
    tray: Tray,
    /// Shown in the tray menu, read without borrowing the App.
    toggle_name: String,
    unlock_color: Rgb,
    lock_color: Rgb,
    quit_color: Rgb,
    glow_width: u32,
}

thread_local! {
    // Separate cells so a hook callback never needs the App borrow.
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static CONTROLLER: RefCell<HWND> = RefCell::new(HWND::default());
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_engine<R>(f: impl FnOnce(&mut Engine) -> R) -> R {
    ENGINE.with_borrow_mut(|e| {
        f(e.as_mut()
            .expect("engine is set before hooks are installed"))
    })
}

fn with_app(f: impl FnOnce(&mut App)) {
    APP.with_borrow_mut(|a| {
        if let Some(a) = a.as_mut() {
            f(a);
        }
    });
}

pub fn run(cfg: Config, opts: Options) -> Result<(), String> {
    let start = if cfg.start_locked {
        Mode::Locked
    } else {
        Mode::Unlocked
    };
    ENGINE.set(Some(Engine::new(
        start,
        cfg.toggle_keys.clone(),
        cfg.quit_keys.clone(),
        cfg.hold,
    )));

    unsafe {
        // Hook callbacks must never be starved, or Windows drops the hook.
        let _ = SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS);
    }
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|e| e.to_string())?
        .into();
    overlay::register_class(instance);
    let controller = create_controller(instance)?;
    CONTROLLER.set(controller);

    let hooks = Hooks::install()?;
    let desktop_hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_DESKTOPSWITCH,
            EVENT_SYSTEM_DESKTOPSWITCH,
            None,
            Some(on_desktop_switch),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    unsafe {
        SetTimer(Some(controller), TIMER_REARM, REARM_MS, None);
        if let Some(f) = opts.failsafe {
            SetTimer(
                Some(controller),
                TIMER_FAILSAFE,
                f.as_millis().min(u32::MAX as u128) as u32,
                None,
            );
        }
    }

    let toggle_name = keys::combo_name(&cfg.toggle_keys);
    let mut app = App {
        controller,
        overlay: Overlay::new(cfg.glow_width),
        anim: Anim::new(),
        animating: false,
        was_holding: false,
        hooks,
        touchpad: cfg.disable_touchpad.then(Touchpad::new),
        tray: Tray::new(
            controller,
            instance,
            start == Mode::Locked,
            toggle_name.clone(),
        ),
        toggle_name,
        unlock_color: cfg.unlock_color,
        lock_color: cfg.lock_color,
        quit_color: cfg.quit_color,
        glow_width: cfg.glow_width,
    };
    let quit = cfg
        .quit_keys
        .as_ref()
        .map_or_else(|| "off".into(), keys::combo_name);
    log::line(&format!(
        "started {}, toggle {}, quit {quit}",
        mode_name(start),
        keys::combo_name(&cfg.toggle_keys)
    ));
    if start == Mode::Locked {
        app.set_touchpad_locked(true);
        app.anim.flash(app.lock_color, Instant::now());
        app.start_animating();
    }
    APP.set(Some(app));

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if !desktop_hook.is_invalid() {
            let _ = UnhookWinEvent(desktop_hook);
        }
    }
    // Dropping the App removes the hooks and the glow windows.
    APP.set(None);
    log::line("exited");
    Ok(())
}

fn create_controller(instance: HINSTANCE) -> Result<HWND, String> {
    let class = w!("DontUseMySystem.Controller");
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(controller_proc),
        hInstance: instance,
        lpszClassName: class,
        ..Default::default()
    };
    unsafe {
        RegisterClassExW(&wc);
        // A hidden top-level window (not message-only) so it receives WM_DISPLAYCHANGE.
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!("DontUseMySystem"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
        .map_err(|e| format!("CreateWindowExW: {e}"))
    }
}

/// Opens the tray menu while unlocked. The menu runs outside any App borrow,
/// since its modal loop keeps dispatching our timers.
fn on_tray(hwnd: HWND, wp: WPARAM, lp: LPARAM) {
    let Some(at) = tray::menu_request(wp, lp) else {
        return;
    };
    if with_engine(|e| e.mode()) == Mode::Locked {
        return;
    }
    let mut toggle = String::new();
    with_app(|a| toggle.clone_from(&a.toggle_name));
    match tray::show_menu(hwnd, at, &toggle) {
        tray::CMD_LOCK => {
            if let Some(ev) = with_engine(Engine::lock) {
                with_app(|a| {
                    log::line("lock from tray");
                    a.apply(ev, Instant::now());
                    a.start_animating();
                });
            }
        }
        tray::CMD_QUIT => {
            log::line("quit from tray");
            unsafe { PostQuitMessage(0) };
        }
        _ => {}
    }
}

unsafe extern "system" fn controller_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == tray::taskbar_created() {
        log::line("taskbar restarted, re-adding tray icon");
        with_app(|a| a.tray.add());
        return LRESULT(0);
    }
    match msg {
        tray::WM_TRAY => on_tray(hwnd, wp, lp),
        WM_APP_WAKE => with_app(App::start_animating),
        WM_TIMER => match wp.0 {
            TIMER_ANIM => with_app(App::frame),
            TIMER_REARM => with_app(|a| a.hooks.rearm()),
            TIMER_FAILSAFE => {
                log::line("failsafe timeout, quitting");
                unsafe { PostQuitMessage(0) };
            }
            _ => {}
        },
        WM_DISPLAYCHANGE => with_app(|a| {
            log::line("display changed, rebuilding glow");
            a.overlay = Overlay::new(a.glow_width);
            a.tray.reload_icons();
        }),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
}

impl App {
    /// Non-blocking: the device work happens on the touchpad worker thread.
    fn set_touchpad_locked(&self, locked: bool) {
        match (&self.touchpad, locked) {
            (Some(t), true) => t.disable(),
            (Some(t), false) => t.restore(),
            (None, _) => {}
        }
    }

    /// Applies a lock or unlock, whether it came from a hold or the tray menu.
    fn apply(&mut self, event: Event, now: Instant) {
        let locked = match event {
            Event::Locked => true,
            Event::Unlocked => false,
            Event::Quit => return,
        };
        log::line(if locked { "locked" } else { "unlocked" });
        if locked {
            // A tray menu left open would sit on screen, unreachable, until unlock.
            unsafe {
                let _ = EndMenu();
            }
        }
        self.set_touchpad_locked(locked);
        self.tray.set_locked(locked);
        let color = if locked {
            self.lock_color
        } else {
            self.unlock_color
        };
        self.anim.flash(color, now);
    }

    fn start_animating(&mut self) {
        if !self.animating {
            unsafe { SetTimer(Some(self.controller), TIMER_ANIM, FRAME_MS, None) };
            self.animating = true;
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let (event, hold, mode) = with_engine(|e| {
            let ev = e.tick(now);
            (ev, e.progress(now), e.mode())
        });
        if let Some(ev) = event {
            if ev == Event::Quit {
                log::line("quit");
                unsafe {
                    let _ = EndMenu();
                    PostQuitMessage(0);
                }
                return;
            }
            self.apply(ev, now);
        }
        match (self.was_holding, hold) {
            (false, Some((a, _))) => log::line(&format!("hold started {a:?}")),
            (true, None) if event.is_none() => log::line("hold cancelled"),
            _ => {}
        }
        self.was_holding = hold.is_some();

        let hold = hold.map(|(action, p)| (self.color_for(action, mode), p));
        match self.anim.step(now, hold) {
            Some(f) => self.overlay.show(f.color, f.intensity),
            None => {
                self.overlay.hide();
                unsafe {
                    let _ = KillTimer(Some(self.controller), TIMER_ANIM);
                }
                self.animating = false;
            }
        }
    }

    fn color_for(&self, action: Action, mode: Mode) -> Rgb {
        match (action, mode) {
            (Action::Quit, _) => self.quit_color,
            (Action::Toggle, Mode::Locked) => self.unlock_color,
            (Action::Toggle, Mode::Unlocked) => self.lock_color,
        }
    }
}

struct Hooks {
    keyboard: HHOOK,
    mouse: HHOOK,
}

impl Hooks {
    fn install() -> Result<Self, String> {
        unsafe {
            let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0)
                .map_err(|e| format!("keyboard hook: {e}"))?;
            let mouse = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) {
                Ok(m) => m,
                Err(e) => {
                    let _ = UnhookWindowsHookEx(keyboard);
                    return Err(format!("mouse hook: {e}"));
                }
            };
            Ok(Self { keyboard, mouse })
        }
    }

    /// Installs fresh hooks before removing the old ones, so there is no gap.
    fn rearm(&mut self) {
        match Self::install() {
            Ok(fresh) => *self = fresh,
            Err(e) => log::line(&format!("rearm failed: {e}")),
        }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.keyboard);
            let _ = UnhookWindowsHookEx(self.mouse);
        }
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        if kb.dwExtraInfo != MASK_TAG {
            let down = matches!(wp.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let v = with_engine(|e| e.on_key(kb.vkCode as u16, down, Instant::now()));
            if v.mask {
                send_mask_key();
            }
            if v.hold_started {
                wake();
            }
            if v.swallow {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let ms = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        let x_button = || {
            if (ms.mouseData >> 16) & 0xFFFF == 1 {
                3
            } else {
                4
            }
        };
        let swallow = with_engine(|e| match wp.0 as u32 {
            WM_LBUTTONDOWN => e.on_button(0, true),
            WM_LBUTTONUP => e.on_button(0, false),
            WM_RBUTTONDOWN => e.on_button(1, true),
            WM_RBUTTONUP => e.on_button(1, false),
            WM_MBUTTONDOWN => e.on_button(2, true),
            WM_MBUTTONUP => e.on_button(2, false),
            WM_XBUTTONDOWN => e.on_button(x_button(), true),
            WM_XBUTTONUP => e.on_button(x_button(), false),
            _ => e.on_motion(),
        });
        if swallow {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

unsafe extern "system" fn on_desktop_switch(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    log::line("desktop switched, clearing held keys");
    with_engine(Engine::reset_keys);
    wake();
}

fn wake() {
    let hwnd = CONTROLLER.with_borrow(|h| *h);
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_WAKE, WPARAM(0), LPARAM(0));
    }
}

fn send_mask_key() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(MASK_VK),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MASK_TAG,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Locked => "locked",
        Mode::Unlocked => "unlocked",
    }
}
