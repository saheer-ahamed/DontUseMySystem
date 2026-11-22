# DontUseMySystem

Blocks every mouse, keyboard and touchpad event on Windows until you hold a key combination.
A glowing border shows the hold progress, then flashes to confirm.

## Download

Get `dontusemysystem.exe` from the [latest release](https://github.com/saheer-ahamed/DontUseMySystem/releases/latest) and run it.
It is a single portable exe for Windows 10 and 11 (x64), with nothing to install.
The exe is not code-signed, so SmartScreen may warn on first run: choose "More info", then "Run anyway".

## Use

- Hold **Ctrl+Alt+U** for 1.2 s to unlock. Hold it again to lock.
- While unlocked, hold **Ctrl+Alt+Shift+Q** to quit, or use the tray icon's menu: **Lock now** or **Quit**.
  The tray icon shows the state (pink closed lock when locked, cyan open lock when unlocked); its menu only opens while unlocked.
- Emergency exit: **Ctrl+Alt+Del** always works, then end the process in Task Manager.
- If the app crashes, Windows removes its hooks and input comes back on its own.
- The touchpad is disabled while locked and enabled again when you unlock or quit.
  This needs administrator rights, so the app asks for them through UAC at launch.
  Decline, or pass `--no-elevate`, and everything else still works, but touchpad gestures get through.
  If the app is killed while locked, the touchpad comes back on the next launch or the next reboot, whichever is first.

## Build and run

```bash
cargo build --release
```

```bash
target/release/dontusemysystem.exe
```

## Install a local build

```bash
powershell -ExecutionPolicy Bypass -File scripts/install.ps1 -Desktop
```

This copies the release build to `%LOCALAPPDATA%\Programs\DontUseMySystem` and adds a Start menu shortcut.
`-Desktop` also adds a desktop shortcut, and `-Startup` launches it at sign-in.
Run it again after a rebuild to update, or pass `-Uninstall` to remove it.
Quit the app first, since the script refuses to replace or remove a running copy.

Settings live in `%APPDATA%\DontUseMySystem\config.toml`, created on first run.
Flags: `--config <path>`, `--unlocked`, `--failsafe <secs>`, `--log <path>`, `--no-elevate`.

## How it works

- `WH_KEYBOARD_LL` and `WH_MOUSE_LL` hooks drop events before any app sees them.
- `src/engine.rs` is the pure state machine (lock state, combo holds, which key-ups to drop), covered by unit tests.
- `src/tray.rs` is the notification-area icon and menu.
  The icons are drawn by `scripts/make-icons.ps1` into `assets/` and compiled into the exe by `build.rs`.
- The glow is four click-through layered windows per monitor; fading only changes the window alpha, so animating costs almost nothing.
- Hooks are re-installed every 3 s, because Windows silently drops a low-level hook whose callback is ever too slow.
- Touchpad gestures go straight to the Windows shell and bypass hooks, so `src/touchpad.rs` disables the touchpad's HID collection (`HID_DEVICE_UP:000D_U:0005`) on a worker thread.
  Only that collection is disabled, never its parent device, which on some laptops also carries the keyboard.
  The disable is not persisted, so a reboot always restores it.

## Known limits

- Ctrl+Alt+Del and Win+L cannot be blocked by any normal app.
- Windows running as administrator ignore hooks from a non-admin process. Run this app as administrator to cover them.
- Touchscreen and pen input are untested and may get through.

## Tests

```bash
cargo test
```

```bash
powershell -ExecutionPolicy Bypass -File scripts/e2e.ps1
```

The end-to-end script runs the real exe and injects input, so your own input is blocked for about 10 s while it runs.
Run it from an elevated PowerShell to include the touchpad checks.

## License

[MIT](LICENSE)
