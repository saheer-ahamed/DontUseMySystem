//! DontUseMySystem: blocks every mouse, keyboard and touchpad event until a
//! key combination is held. Hold it again to re-lock.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod config;
mod engine;
mod keys;
mod log;
mod overlay;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod touchpad;
#[cfg(windows)]
mod tray;

use std::path::PathBuf;
use std::time::Duration;

const USAGE: &str =
    "Usage: dontusemysystem [--config <path>] [--unlocked] [--failsafe <seconds>] [--log <path>]

  --config <path>     Config file (default: %APPDATA%\\DontUseMySystem\\config.toml)
  --unlocked          Start unlocked, overriding start_locked
  --failsafe <secs>   Quit automatically after this many seconds (for testing)
  --log <path>        Write state changes to a log file
  --no-elevate        Never ask for administrator (touchpad gestures stay unblocked)";

struct Args {
    config: PathBuf,
    unlocked: bool,
    failsafe: Option<Duration>,
    log: Option<PathBuf>,
    no_elevate: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        config: config::Config::default_path(),
        unlocked: false,
        failsafe: None,
        log: None,
        no_elevate: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--config" => args.config = value()?.into(),
            "--unlocked" => args.unlocked = true,
            "--failsafe" => {
                let v = value()?;
                let secs: u64 = v
                    .parse()
                    .map_err(|_| format!("--failsafe: not a number: {v}"))?;
                args.failsafe = Some(Duration::from_secs(secs));
            }
            "--log" => args.log = Some(value()?.into()),
            "--no-elevate" => args.no_elevate = true,
            "--help" | "-h" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    Ok(args)
}

/// Quotes one argument so `CommandLineToArgvW` reads it back unchanged.
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // Backslashes before a quote are doubled, then the quote is escaped.
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    // Trailing backslashes are doubled so they do not escape the closing quote.
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::quote_arg;

    #[test]
    fn quotes_like_command_line_to_argv() {
        assert_eq!(quote_arg("--unlocked"), "--unlocked");
        assert_eq!(quote_arg(r"C:\a b\c.toml"), r#""C:\a b\c.toml""#);
        assert_eq!(quote_arg(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_arg(""), r#""""#);
    }
}

#[cfg(windows)]
fn main() {
    if let Err(e) = real_main() {
        win::error_box(&e);
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn real_main() -> Result<(), String> {
    let args = parse_args()?;
    if let Some(path) = &args.log {
        log::init(path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    win::set_dpi_aware();
    let mut cfg = config::Config::load(&args.config)?;
    if args.unlocked {
        cfg.start_locked = false;
    }
    // Disabling the touchpad device needs admin, so relaunch elevated first.
    if cfg.disable_touchpad && !args.no_elevate && !win::is_elevated() && touchpad::present() {
        let mut child: Vec<String> = std::env::args().skip(1).collect();
        child.push("--no-elevate".into());
        let params = child
            .iter()
            .map(|a| quote_arg(a))
            .collect::<Vec<_>>()
            .join(" ");
        match win::relaunch_elevated(&params) {
            Ok(true) => {
                log::line("relaunched as administrator");
                return Ok(());
            }
            Ok(false) => log::line("administrator declined: touchpad gestures will not be blocked"),
            Err(e) => log::line(&format!("could not relaunch as administrator: {e}")),
        }
    }
    let _instance = win::single_instance()?;
    app::run(
        cfg,
        app::Options {
            failsafe: args.failsafe,
        },
    )
}

#[cfg(not(windows))]
fn main() {
    eprintln!("DontUseMySystem only runs on Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::{HSTRING, w};

    pub fn is_elevated() -> bool {
        use windows::Win32::Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some((&raw mut elevation).cast()),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok && elevation.TokenIsElevated != 0
        }
    }

    /// Starts this exe again through UAC. `Ok(false)` means the user declined.
    pub fn relaunch_elevated(params: &str) -> Result<bool, String> {
        use windows::Win32::Foundation::ERROR_CANCELLED;
        use windows::Win32::UI::Shell::{SHELLEXECUTEINFOW, ShellExecuteExW};
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::HRESULT;

        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = HSTRING::from(exe.as_os_str());
        let params = HSTRING::from(params);
        let dir = HSTRING::from(std::env::current_dir().unwrap_or_default().as_os_str());
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            lpVerb: w!("runas"),
            lpFile: windows::core::PCWSTR(exe.as_ptr()),
            lpParameters: windows::core::PCWSTR(params.as_ptr()),
            lpDirectory: windows::core::PCWSTR(dir.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        match unsafe { ShellExecuteExW(&mut info) } {
            Ok(()) => Ok(true),
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn set_dpi_aware() {
        // Physical pixels, so the glow hugs every monitor edge at any scaling.
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }

    pub struct Instance(HANDLE);

    impl Drop for Instance {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    pub fn single_instance() -> Result<Instance, String> {
        let handle = unsafe { CreateMutexW(None, false, w!("Local\\DontUseMySystem.Instance")) }
            .map_err(|e| format!("CreateMutexW: {e}"))?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let _ = unsafe { CloseHandle(handle) };
            return Err("DontUseMySystem is already running.".into());
        }
        Ok(Instance(handle))
    }

    pub fn error_box(msg: &str) {
        eprintln!("{msg}");
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(msg),
                w!("DontUseMySystem"),
                MB_OK | MB_ICONERROR,
            )
        };
    }
}
