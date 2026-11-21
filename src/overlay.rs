//! Glowing border drawn with click-through, per-pixel-alpha layered windows.
//!
//! Each monitor gets four thin edge windows instead of one full-screen window,
//! so the bitmaps stay tiny and nothing covers the screen when idle. Fading is
//! done with `SourceConstantAlpha`, so animating never re-renders pixels.

use std::ffi::c_void;
use std::ptr::null_mut;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO,
    SelectObject,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_TOPMOST, RegisterClassExW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{BOOL, w};

use crate::config::Rgb;

const CLASS: windows::core::PCWSTR = w!("DontUseMySystem.Glow");

pub fn register_class(instance: windows::Win32::Foundation::HINSTANCE) {
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(glow_proc),
        hInstance: instance,
        lpszClassName: CLASS,
        ..Default::default()
    };
    unsafe { RegisterClassExW(&wc) };
}

unsafe extern "system" fn glow_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// One edge strip of one monitor.
struct Band {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    /// Position of this strip inside its monitor.
    origin: (i32, i32),
    size: (i32, i32),
    monitor_size: (i32, i32),
    width: i32,
}

pub struct Overlay {
    bands: Vec<Band>,
    color: Option<Rgb>,
    visible: bool,
}

impl Overlay {
    /// Builds edge windows for every monitor. `glow_width` is at 100% scaling.
    pub fn new(glow_width: u32) -> Self {
        let mut bands = Vec::new();
        for (rect, dpi) in monitors() {
            let width = (glow_width as f32 * dpi as f32 / 96.0).round() as i32;
            let (mw, mh) = (rect.right - rect.left, rect.bottom - rect.top);
            let width = width.min(mw / 4).min(mh / 4).max(1);
            let strips = [
                (0, 0, mw, width),
                (0, mh - width, mw, width),
                (0, width, width, mh - 2 * width),
                (mw - width, width, width, mh - 2 * width),
            ];
            for (x, y, w, h) in strips {
                if let Some(b) = Band::new((rect.left, rect.top), (x, y), (w, h), (mw, mh), width) {
                    bands.push(b);
                }
            }
        }
        Self {
            bands,
            color: None,
            visible: false,
        }
    }

    /// Shows the glow at `intensity` (0 to 1); 0 hides it.
    pub fn show(&mut self, color: Rgb, intensity: f32) {
        let alpha = (intensity.clamp(0.0, 1.0) * 255.0).round() as u8;
        if alpha == 0 {
            self.hide();
            return;
        }
        let recolor = self.color != Some(color);
        if recolor {
            for b in &self.bands {
                b.render(color);
            }
            self.color = Some(color);
        }
        // The bitmap only needs re-uploading when it changed or the window was hidden.
        let full = recolor || !self.visible;
        for b in &self.bands {
            b.set_alpha(alpha, full);
        }
        if !self.visible {
            for b in &self.bands {
                unsafe {
                    let _ = ShowWindow(b.hwnd, SW_SHOWNOACTIVATE);
                    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
                    let _ = SetWindowPos(b.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
                }
            }
            self.visible = true;
        }
    }

    pub fn hide(&mut self) {
        if self.visible {
            for b in &self.bands {
                unsafe {
                    let _ = ShowWindow(b.hwnd, SW_HIDE);
                }
            }
            self.visible = false;
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        for b in self.bands.drain(..) {
            unsafe {
                let _ = DestroyWindow(b.hwnd);
                SelectObject(b.dc, b.old);
                let _ = DeleteObject(b.bitmap.into());
                let _ = DeleteDC(b.dc);
            }
        }
    }
}

impl Band {
    fn new(
        monitor_pos: (i32, i32),
        origin: (i32, i32),
        size: (i32, i32),
        monitor_size: (i32, i32),
        width: i32,
    ) -> Option<Self> {
        if size.0 <= 0 || size.1 <= 0 {
            return None;
        }
        unsafe {
            let ex = WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE;
            let hwnd = CreateWindowExW(
                ex,
                CLASS,
                w!(""),
                WS_POPUP,
                monitor_pos.0 + origin.0,
                monitor_pos.1 + origin.1,
                size.0,
                size.1,
                None,
                None,
                None,
                None,
            )
            .ok()?;
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size.0,
                    biHeight: -size.1, // top-down rows
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = null_mut();
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                let _ = DeleteDC(dc);
                let _ = DestroyWindow(hwnd);
                return None;
            };
            let old = SelectObject(dc, bitmap.into());
            Some(Self {
                hwnd,
                dc,
                bitmap,
                old,
                bits: bits.cast(),
                origin,
                size,
                monitor_size,
                width,
            })
        }
    }

    /// Writes premultiplied BGRA pixels for the glow at full strength.
    fn render(&self, color: Rgb) {
        let (w, h) = self.size;
        let (mw, mh) = self.monitor_size;
        let px = unsafe { std::slice::from_raw_parts_mut(self.bits, (w * h) as usize) };
        let width = self.width as f32;
        for y in 0..h {
            for x in 0..w {
                let d = edge_distance(self.origin.0 + x, self.origin.1 + y, mw, mh, self.width);
                px[(y * w + x) as usize] = glow_pixel(color, d, width);
            }
        }
    }

    /// Pushes the bitmap (when `full`) or only a new overall alpha.
    fn set_alpha(&self, alpha: u8, full: bool) {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let size = SIZE {
            cx: self.size.0,
            cy: self.size.1,
        };
        let src = POINT { x: 0, y: 0 };
        unsafe {
            let _ = if full {
                UpdateLayeredWindow(
                    self.hwnd,
                    None,
                    None,
                    Some(&size),
                    Some(self.dc),
                    Some(&src),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                )
            } else {
                UpdateLayeredWindow(
                    self.hwnd,
                    None,
                    None,
                    None,
                    None,
                    None,
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                )
            };
        }
    }
}

/// Distance from the monitor edge, measured as `width` minus the distance to
/// the glow's inner rounded rectangle. Along an edge this is the plain distance;
/// in corners it curves, so there is no diagonal crease where two edges meet.
fn edge_distance(x: i32, y: i32, mw: i32, mh: i32, width: i32) -> f32 {
    let qx = (width - x).max(x - (mw - 1 - width)).max(0) as f32;
    let qy = (width - y).max(y - (mh - 1 - width)).max(0) as f32;
    (width as f32 - qx.hypot(qy)).max(0.0)
}

/// Soft falloff from the screen edge, with a whiter hot rim right at the edge.
fn glow_pixel(c: Rgb, d: f32, width: f32) -> u32 {
    let t = d / width;
    if t >= 1.0 {
        return 0;
    }
    let fade = 1.0 - t;
    let a = (0.9 * (-5.5 * t).exp() + 0.35 * fade * fade * fade).min(1.0) * smooth_edge(fade);
    let rim = 0.55 * (-d / 1.6).exp();
    let ch = |v: u8| {
        let lit = f32::from(v) + (255.0 - f32::from(v)) * rim;
        (lit * a).round() as u32
    };
    let a8 = (a * 255.0).round() as u32;
    (a8 << 24) | (ch(c.0) << 16) | (ch(c.1) << 8) | ch(c.2)
}

/// Eases the last few percent to zero so the inner edge of the glow never shows a seam.
fn smooth_edge(fade: f32) -> f32 {
    (fade / 0.15).min(1.0)
}

fn monitors() -> Vec<(RECT, u32)> {
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<(RECT, u32)>) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(m, &mut info) }.as_bool() {
            let (mut dx, mut dy) = (96, 96);
            let _ = unsafe { GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
            out.push((info.rcMonitor, dx));
        }
        BOOL(1)
    }
    let mut out: Vec<(RECT, u32)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_distance_is_plain_on_edges_and_round_in_corners() {
        let d = |x, y| edge_distance(x, y, 1000, 800, 40);
        assert_eq!(d(10, 400), 10.0);
        assert_eq!(d(500, 795), 4.0);
        assert_eq!(d(500, 400), 40.0, "interior is beyond the glow");
        assert_eq!(d(0, 0), 0.0);
        // Symmetric across the corner diagonal and continuous with the edges.
        assert_eq!(d(15, 25), d(25, 15));
        assert_eq!(d(40, 10), 10.0);
    }

    #[test]
    fn glow_is_premultiplied_and_fades_inward() {
        let c = Rgb(0, 229, 255);
        let alpha = |p: u32| p >> 24;
        let edge = glow_pixel(c, 0.0, 36.0);
        let mid = glow_pixel(c, 12.0, 36.0);
        assert!(alpha(edge) > alpha(mid) && alpha(mid) > 0);
        assert_eq!(glow_pixel(c, 36.0, 36.0), 0);
        assert!(
            alpha(glow_pixel(c, 35.0, 36.0)) < 4,
            "no visible seam at the inner edge"
        );
        for p in [edge, mid] {
            for shift in [0, 8, 16] {
                assert!((p >> shift) & 0xFF <= alpha(p), "channel exceeds alpha");
            }
        }
    }
}
