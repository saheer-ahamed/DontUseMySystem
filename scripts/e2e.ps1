# End-to-end test: runs the real exe and drives it with injected input (SendInput).
# WARNING: your own mouse and keyboard are blocked while it runs (at most ~20 s, then the
# app's failsafe quits it). Ctrl+Alt+Del always works as an escape.
#
#   powershell -ExecutionPolicy Bypass -File scripts\e2e.ps1 [-Exe path] [-ShotDir dir]

param(
    [string]$Exe = "$PSScriptRoot\..\target\release\dontusemysystem.exe",
    [string]$ShotDir = ""
)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class Input {
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; public int pad1, pad2; }
    [StructLayout(LayoutKind.Explicit)] struct INPUT {
        [FieldOffset(0)] public uint type;
        [FieldOffset(8)] public MOUSEINPUT mi;
        [FieldOffset(8)] public KEYBDINPUT ki;
    }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] static extern short GetKeyState(int vk);
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();

    static void Send(INPUT i) {
        if (SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT))) != 1) throw new Exception("SendInput failed");
    }
    public static void Key(ushort vk, bool down) {
        var i = new INPUT { type = 1 };
        i.ki.vk = vk; i.ki.flags = down ? 0u : 2u;
        Send(i);
    }
    public static void Move(int dx, int dy) {
        var i = new INPUT { type = 0 };
        i.mi.dx = dx; i.mi.dy = dy; i.mi.flags = 1; // MOUSEEVENTF_MOVE (relative)
        Send(i);
    }
    public static POINT Cursor() { POINT p; GetCursorPos(out p); return p; }

    // Tray: find our icon, click it like a user would, and see whether the menu opened.
    [StructLayout(LayoutKind.Sequential)] struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct NOTIFYICONIDENTIFIER { public uint cbSize; public IntPtr hWnd; public uint uID; public Guid guid; }
    [DllImport("shell32.dll")] static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER id, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] static extern bool AllowSetForegroundWindow(int pid);
    static IntPtr Controller() { return FindWindow("DontUseMySystem.Controller", null); }
    // Null when the icon is not registered with the shell.
    public static int[] TrayRect() {
        var id = new NOTIFYICONIDENTIFIER { hWnd = Controller(), uID = 1 };
        id.cbSize = (uint)Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER));
        RECT r;
        if (id.hWnd == IntPtr.Zero || Shell_NotifyIconGetRect(ref id, out r) != 0) return null;
        return new[] { r.L, r.T, r.R, r.B };
    }
    public static bool MenuOpen() {
        for (IntPtr h = FindWindowEx(IntPtr.Zero, IntPtr.Zero, "#32768", null); h != IntPtr.Zero; h = FindWindowEx(IntPtr.Zero, h, "#32768", null))
            if (IsWindowVisible(h)) return true;
        return false;
    }
    // Clicks the centre of the icon's rect, if that is on the taskbar or its overflow flyout.
    // Hidden icons report the overflow chevron's rect until the flyout is open.
    public static bool ClickTray(bool right) {
        SetProcessDPIAware();
        var r = TrayRect();
        if (r == null) return false;
        var c = new POINT { X = (r[0] + r[2]) / 2, Y = (r[1] + r[3]) / 2 };
        SetCursorPos(c.X, c.Y);
        var i = new INPUT { type = 0 };
        i.mi.flags = right ? 0x8u : 0x2u; Send(i);
        i.mi.flags = right ? 0x10u : 0x4u; Send(i);
        return true;
    }
    // What the shell sends for a right click, used when the icon is in the overflow flyout.
    public static void OpenTrayMenu(int pid) {
        AllowSetForegroundWindow(pid);
        var p = Cursor();
        PostMessage(Controller(), 0x8002, (IntPtr)((p.Y << 16) | (p.X & 0xFFFF)), (IntPtr)(0x7B | (1 << 16)));
    }

    public static bool ScrollLockOn() { return (GetKeyState(0x91) & 1) != 0; }
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, uint rop);
    // CAPTUREBLT is required, or layered windows (the glow) are left out of the capture.
    public static void Shot(string path) {
        SetProcessDPIAware();
        var b = System.Windows.Forms.SystemInformation.VirtualScreen;
        using (var bmp = new Bitmap(b.Width, b.Height))
        using (var g = Graphics.FromImage(bmp)) {
            IntPtr screen = GetDC(IntPtr.Zero), dst = g.GetHdc();
            BitBlt(dst, 0, 0, b.Width, b.Height, screen, b.Left, b.Top, 0x00CC0020 | 0x40000000);
            g.ReleaseHdc(dst); ReleaseDC(IntPtr.Zero, screen);
            bmp.Save(path, ImageFormat.Png);
        }
    }
}
'@ -ReferencedAssemblies System.Drawing, System.Windows.Forms

$CTRL = 0xA2; $ALT = 0xA4; $SHIFT = 0xA0; $U = 0x55; $Q = 0x51; $L = 0x4C; $ESC = 0x1B; $SCROLL = 0x91
$failures = @()
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "  ok   $what" } else { Write-Host "  FAIL $what"; $script:failures += $what }
}
function MouseMoves {
    $a = [Input]::Cursor(); [Input]::Move(40, 0); Start-Sleep -Milliseconds 80
    $b = [Input]::Cursor(); [Input]::Move(-40, 0); Start-Sleep -Milliseconds 80
    return $a.X -ne $b.X
}
function KeyArrives {
    # Scroll Lock is harmless to toggle; its toggle state only flips if the key reached Windows.
    $before = [Input]::ScrollLockOn()
    [Input]::Key($SCROLL, $true); [Input]::Key($SCROLL, $false); Start-Sleep -Milliseconds 80
    $changed = [Input]::ScrollLockOn() -ne $before
    if ($changed) { [Input]::Key($SCROLL, $true); [Input]::Key($SCROLL, $false); Start-Sleep -Milliseconds 80 }
    return $changed
}
# Opens the menu the way a user does: right-click the icon, first opening the
# overflow flyout if the icon is hidden there. Posting the shell's callback is
# the last resort, for taskbars where the icon cannot be reached by clicking.
function TrayMenu([System.Diagnostics.Process]$proc) {
    $before = [Input]::TrayRect() -join ','
    if ([Input]::ClickTray($true)) {
        Start-Sleep -Milliseconds 300
        if ([Input]::MenuOpen()) { Write-Host "       (menu opened by right-clicking the icon)"; return $true }
        if ([Input]::ClickTray($false)) { Start-Sleep -Milliseconds 600 }
        if (([Input]::TrayRect() -join ',') -ne $before -and [Input]::ClickTray($true)) {
            Start-Sleep -Milliseconds 300
            if ([Input]::MenuOpen()) { Write-Host "       (menu opened by right-clicking the icon in the overflow flyout)"; return $true }
        }
        Tap $ESC
    }
    [Input]::OpenTrayMenu($proc.Id)
    Start-Sleep -Milliseconds 300
    $open = [Input]::MenuOpen()
    if ($open) { Write-Host "       (menu opened by posting the shell callback)" }
    return $open
}
function Tap([int]$vk) { [Input]::Key($vk, $true); [Input]::Key($vk, $false); Start-Sleep -Milliseconds 250 }
function Hold([int[]]$keys, [int]$ms, [string]$shot = "") {
    foreach ($k in $keys) { [Input]::Key($k, $true) }
    try {
        if ($shot -and $ShotDir) { Start-Sleep -Milliseconds ([int]($ms * 0.7)); [Input]::Shot("$ShotDir\$shot"); Start-Sleep -Milliseconds ([int]($ms * 0.3)) }
        else { Start-Sleep -Milliseconds $ms }
    }
    finally {
        [array]::Reverse($keys); foreach ($k in $keys) { [Input]::Key($k, $false) }
    }
    Start-Sleep -Milliseconds 120
}

$Exe = (Resolve-Path $Exe).Path
$log = Join-Path $env:TEMP "dums-e2e.log"
Remove-Item $log -ErrorAction SilentlyContinue

Write-Host "baseline (app not running)"
Check (MouseMoves) "injected mouse moves the cursor"
Check (KeyArrives) "injected key reaches Windows"

# Touchpad blocking disables the device, which needs admin. Unelevated runs skip it
# (--no-elevate keeps the app from raising a UAC prompt for every launch).
$elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$touchpads = @(Get-PnpDevice -PresentOnly -Class HIDClass | Where-Object {
    (Get-PnpDeviceProperty -InstanceId $_.InstanceId -KeyName DEVPKEY_Device_HardwareIds).Data -contains 'HID_DEVICE_UP:000D_U:0005'
} | ForEach-Object InstanceId)
function TouchpadOn { foreach ($id in $touchpads) { if ((Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_ProblemCode).Data -eq 22) { return $false } }; return $true }
$tp = $elevated -and $touchpads.Count -gt 0
$config = "$PSScriptRoot\e2e-config.toml"
if ($tp) {
    Check (TouchpadOn) "touchpad on before the test"
    $config = Join-Path $env:TEMP "dums-e2e-touchpad.toml"
    (Get-Content "$PSScriptRoot\e2e-config.toml") -replace 'disable_touchpad = false', 'disable_touchpad = true' | Set-Content -Encoding utf8 $config
} else {
    Write-Host "  skip touchpad checks (run elevated on a machine with a touchpad to include them)"
}
function TouchpadIs([bool]$on, [string]$what) { if ($tp) { Check ((TouchpadOn) -eq $on) $what } }
function Launch([string[]]$extra) {
    Start-Process $Exe -ArgumentList (@("--config", "`"$config`"", "--no-elevate") + $extra) -PassThru
}

$p = Launch @("--failsafe", "20", "--log", "`"$log`"")
try {
    Start-Sleep -Milliseconds 700
    Write-Host "locked"
    Check (-not (MouseMoves)) "mouse blocked"
    Check (-not (KeyArrives)) "keyboard blocked"
    TouchpadIs $false "touchpad switched off"

    Hold @($CTRL, $ALT, $U) 400
    Check (-not (MouseMoves)) "short hold does not unlock"

    Hold @($CTRL, $ALT, $SHIFT, $Q) 1000
    Check (-not $p.HasExited) "quit combo ignored while locked"

    Hold @($CTRL, $ALT, $U) 1000 "hold-unlock.png"
    Write-Host "unlocked"
    Check (MouseMoves) "mouse works"
    Check (KeyArrives) "keyboard works"
    TouchpadIs $true "touchpad back on"

    Hold @($CTRL, $ALT, $U) 1000 "hold-lock.png"
    Write-Host "re-locked"
    Check (-not (MouseMoves)) "mouse blocked again"
    TouchpadIs $false "touchpad off again"

    Hold @($CTRL, $ALT, $U) 1000
    Hold @($CTRL, $ALT, $SHIFT, $Q) 1000
    Check ($p.WaitForExit(1500)) "quit combo exits while unlocked"
    Check (MouseMoves) "input works after exit"
    TouchpadIs $true "touchpad on after exit"

    Write-Host "tray"
    $t = Launch @("--unlocked", "--failsafe", "20", "--log", "`"$log.tray`"")
    Start-Sleep -Milliseconds 700
    Check ($null -ne [Input]::TrayRect()) "tray icon registered"
    Check (TrayMenu $t) "tray menu opens while unlocked"
    Tap $L
    Check (-not [Input]::MenuOpen()) "menu closes after Lock now"
    Check (-not (MouseMoves)) "Lock now locks"
    Check (-not (TrayMenu $t)) "tray menu does not open while locked"
    Hold @($CTRL, $ALT, $U) 1000
    Check (TrayMenu $t) "tray menu opens again after unlock"
    Tap $Q
    Check ($t.WaitForExit(1500)) "Quit from tray exits"
    Check ($null -eq [Input]::TrayRect()) "tray icon removed on exit"
    if (-not $t.HasExited) { $t.Kill() }
    if ([Input]::MenuOpen()) { Tap $ESC }

    # Killed while locked: the touchpad stays off until the next launch restores it.
    if ($tp) {
        $k = Launch @("--failsafe", "10", "--log", "`"$log.2`"")
        Start-Sleep -Milliseconds 700
        $k.Kill(); $k.WaitForExit()
        TouchpadIs $false "touchpad still off after a kill (expected)"
        $r = Launch @("--unlocked", "--failsafe", "1", "--log", "`"$log.3`"")
        Check ($r.WaitForExit(5000)) "relaunch runs and exits"
        TouchpadIs $true "relaunch turned the touchpad back on"
    }
}
finally {
    if (-not $p.HasExited) { $p.Kill() }
    if ($tp -and -not (TouchpadOn)) { Write-Host "  restoring touchpad"; (Launch @("--unlocked", "--failsafe", "1")).WaitForExit() }
}
Write-Host "`nlog:"; Get-Content $log, "$log.tray" -ErrorAction SilentlyContinue | ForEach-Object { "  $_" }
if ($failures.Count) { Write-Host "`n$($failures.Count) FAILED"; exit 1 } else { Write-Host "`nall passed" }
