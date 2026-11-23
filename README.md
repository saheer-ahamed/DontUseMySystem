# DontUseMySystem

Blocks the mouse, keyboard and touchpad on Windows until you hold a shortcut, without hiding your screen.

## Why

The Windows lock screen hides what you are doing.
We wanted the opposite: leave the screen in view, but let nobody touch it.
We also did not want a stray click or key press to stop anything that is running, so whatever was in progress carries on while the input is locked.

## Get started

1. Download `dontusemysystem.exe` from the [latest release](https://github.com/saheer-ahamed/DontUseMySystem/releases/latest).
   It runs on 64-bit Windows 10 and 11, with nothing to install.
2. Run it and approve the administrator prompt.
   The exe is not code-signed, so if SmartScreen warns you, choose **More info**, then **Run anyway**.
3. Input is now locked.
   Hold **Ctrl+Alt+U** until the border flashes to unlock.

## Controls

| To | Do this |
| --- | --- |
| Unlock | Hold **Ctrl+Alt+U** for 1.2 seconds |
| Lock again | Hold **Ctrl+Alt+U** for 1.2 seconds, or tray icon > **Lock now** |
| Quit (while unlocked) | Hold **Ctrl+Alt+Shift+Q** for 1.2 seconds, or tray icon > **Quit** |
| Get out in an emergency | Press **Ctrl+Alt+Del**, open Task Manager and end the app |

The border glows brighter as you hold, and letting go early cancels.
The tray icon is a pink closed lock while locked and a cyan open lock while unlocked.
Its menu opens only while unlocked.

## Settings

Edit `%APPDATA%\DontUseMySystem\config.toml`, then restart the app.
The file is created on first run.

| Setting | Default | What it does |
| --- | --- | --- |
| `toggle_keys` | `["Ctrl", "Alt", "U"]` | Shortcut to lock and unlock |
| `quit_keys` | `["Ctrl", "Alt", "Shift", "Q"]` | Shortcut to quit; delete the line to turn it off |
| `hold_ms` | `1200` | How long to hold, from 200 to 10000 milliseconds |
| `start_locked` | `true` | Lock as soon as the app starts |
| `disable_touchpad` | `true` | Turn the touchpad off while locked |
| `unlock_color`, `lock_color`, `quit_color` | `#00E5FF`, `#FF3D7F`, `#FFB020` | Border colors |
| `glow_width` | `36` | Border width in pixels at 100% display scaling |

## Good to know

- Administrator rights are needed to turn the touchpad off and to block input to apps running as administrator.
  If you decline the prompt, everything else still works.
- If the app is killed while locked, the mouse and keyboard come back at once.
  The touchpad comes back on the next launch or restart.
- The app does not keep the PC awake, so your Windows sleep settings still apply.
- Windows does not let any app block Ctrl+Alt+Del or Win+L.
- Touchscreens and pens are untested and may not be blocked.
- This keeps hands off your PC, but it is not a replacement for the Windows lock screen.

## Tech

A single Rust exe that calls the Win32 API directly, with no UI framework.

- **Input:** low-level keyboard and mouse hooks drop events before other apps see them.
- **Touchpad:** gestures bypass those hooks, so the touchpad device is disabled while locked.
- **Border:** click-through layered windows on every monitor.

To build it, run `cargo build --release`.
To test it, run `cargo test`, or `scripts/e2e.ps1` for a run with real input.

## License

[MIT](LICENSE)
