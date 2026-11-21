//! Disables the precision touchpad device while locked.
//!
//! Three- and four-finger gestures go from the touchpad straight to the
//! Windows shell, never as mouse messages, so the low-level mouse hook cannot
//! drop them. There is no user-mode switch for the touchpad either
//! (`TOUCHPAD_PARAMETERS::touchpadEnabled` is a read-only status bit), so the
//! only way to stop gestures is to disable the device, which needs admin.
//!
//! Only the touchpad's own HID collection (usage page 0x0D, usage 0x05) is
//! disabled, never its parent device, because some laptops put the keyboard
//! on the same parent and we must never disable the keys needed to unlock.
//!
//! The disable is not persisted (no `CM_DISABLE_PERSIST`), so a reboot always
//! brings the touchpad back. A marker file lists what we disabled, so the next
//! launch re-enables it if the app was ever killed while locked.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DEVNODE_STATUS_FLAGS, CM_DISABLE_UI_NOT_OK, CM_DRP_HARDWAREID, CM_Disable_DevNode,
    CM_Enable_DevNode, CM_GETIDLIST_FILTER_ENUMERATOR, CM_GETIDLIST_FILTER_PRESENT,
    CM_Get_DevNode_Registry_PropertyW, CM_Get_DevNode_Status, CM_Get_Device_ID_List_SizeW,
    CM_Get_Device_ID_ListW, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CM_PROB,
    CM_PROB_DISABLED, CR_SUCCESS, DN_HAS_PROBLEM,
};
use windows::core::{HSTRING, w};

use crate::log;

/// Hardware ID Windows gives every HID touch pad top-level collection.
const TOUCHPAD_HWID: &str = "HID_DEVICE_UP:000D_U:0005";

/// Handle to the touchpad worker thread. Dropping it re-enables the touchpad
/// and waits until that is done.
///
/// Disabling a device takes hundreds of milliseconds. Doing it on the hook
/// thread would stall the low-level hooks past `LowLevelHooksTimeout`, which
/// lets input through and can make Windows drop the hooks, so it runs here.
pub struct Touchpad {
    tx: Option<Sender<bool>>,
    worker: Option<JoinHandle<()>>,
}

impl Touchpad {
    /// Also re-enables touchpads left disabled by an earlier run that was killed.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<bool>();
        let worker = std::thread::spawn(move || {
            let mut devices = Devices::recover();
            for locked in rx {
                if locked {
                    devices.disable();
                } else {
                    devices.restore();
                }
            }
            devices.restore();
        });
        Self {
            tx: Some(tx),
            worker: Some(worker),
        }
    }

    pub fn disable(&self) {
        self.send(true);
    }

    pub fn restore(&self) {
        self.send(false);
    }

    fn send(&self, locked: bool) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(locked);
        }
    }
}

impl Drop for Touchpad {
    fn drop(&mut self) {
        // Closing the channel ends the worker's loop, which restores the touchpad.
        self.tx.take();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// The touchpads we disabled. Lives on the worker thread only.
struct Devices {
    disabled: Vec<String>,
}

impl Devices {
    fn recover() -> Self {
        if let Ok(text) = std::fs::read_to_string(marker()) {
            for id in text.lines().filter(|l| !l.is_empty()) {
                log::line(&format!(
                    "re-enabling touchpad left off by an earlier run: {id}"
                ));
                set_enabled(id, true);
            }
            let _ = std::fs::remove_file(marker());
        }
        Self {
            disabled: Vec::new(),
        }
    }

    fn disable(&mut self) {
        if !self.disabled.is_empty() {
            return;
        }
        // Skip touchpads the user already disabled: we must not enable those later.
        let targets: Vec<String> = find().into_iter().filter(|id| !is_disabled(id)).collect();
        if targets.is_empty() {
            return;
        }
        // Marker first, so a crash between the two steps still recovers.
        let _ = write_marker(&targets);
        self.disabled = targets
            .into_iter()
            .filter(|id| set_enabled(id, false))
            .collect();
        if self.disabled.is_empty() {
            let _ = std::fs::remove_file(marker());
        } else {
            let _ = write_marker(&self.disabled);
        }
    }

    fn restore(&mut self) {
        if self.disabled.is_empty() {
            return;
        }
        for id in self.disabled.drain(..) {
            set_enabled(&id, true);
        }
        let _ = std::fs::remove_file(marker());
    }
}

/// Whether this machine has a precision touchpad. Works without admin.
pub fn present() -> bool {
    !find().is_empty()
}

/// Instance IDs of present touchpad collections.
fn find() -> Vec<String> {
    let filter = w!("HID");
    let flags = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT;
    let mut len = 0u32;
    if unsafe { CM_Get_Device_ID_List_SizeW(&mut len, filter, flags) } != CR_SUCCESS {
        return Vec::new();
    }
    let mut buf = vec![0u16; len as usize];
    if unsafe { CM_Get_Device_ID_ListW(filter, &mut buf, flags) } != CR_SUCCESS {
        return Vec::new();
    }
    multi_sz(&buf)
        .into_iter()
        .filter(|id| locate(id).is_some_and(|dev| is_touchpad(&hardware_ids(dev))))
        .collect()
}

fn locate(id: &str) -> Option<u32> {
    let mut dev = 0u32;
    let r = unsafe { CM_Locate_DevNodeW(&mut dev, &HSTRING::from(id), CM_LOCATE_DEVNODE_NORMAL) };
    (r == CR_SUCCESS).then_some(dev)
}

fn hardware_ids(dev: u32) -> Vec<String> {
    let mut len = 0u32;
    unsafe { CM_Get_DevNode_Registry_PropertyW(dev, CM_DRP_HARDWAREID, None, None, &mut len, 0) };
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; len.div_ceil(2) as usize];
    let r = unsafe {
        CM_Get_DevNode_Registry_PropertyW(
            dev,
            CM_DRP_HARDWAREID,
            None,
            Some(buf.as_mut_ptr().cast()),
            &mut len,
            0,
        )
    };
    if r == CR_SUCCESS {
        multi_sz(&buf)
    } else {
        Vec::new()
    }
}

fn is_disabled(id: &str) -> bool {
    let Some(dev) = locate(id) else { return false };
    let mut status = CM_DEVNODE_STATUS_FLAGS(0);
    let mut problem = CM_PROB(0);
    let r = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, dev, 0) };
    r == CR_SUCCESS && status.0 & DN_HAS_PROBLEM.0 != 0 && problem == CM_PROB_DISABLED
}

fn set_enabled(id: &str, enabled: bool) -> bool {
    let Some(dev) = locate(id) else {
        log::line(&format!("touchpad not found: {id}"));
        return false;
    };
    let r = unsafe {
        if enabled {
            CM_Enable_DevNode(dev, 0)
        } else {
            CM_Disable_DevNode(dev, CM_DISABLE_UI_NOT_OK)
        }
    };
    let what = if enabled { "on" } else { "off" };
    if r == CR_SUCCESS {
        log::line(&format!("touchpad {what}"));
        true
    } else {
        // 0x33 is CR_ACCESS_DENIED: the app is not running as administrator.
        log::line(&format!("touchpad {what} failed: CONFIGRET 0x{:X}", r.0));
        false
    }
}

fn is_touchpad(hardware_ids: &[String]) -> bool {
    hardware_ids
        .iter()
        .any(|h| h.eq_ignore_ascii_case(TOUCHPAD_HWID))
}

/// Splits a double-NUL-terminated UTF-16 string list.
fn multi_sz(buf: &[u16]) -> Vec<String> {
    buf.split(|&c| c == 0)
        .take_while(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

fn marker() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
    base.join("DontUseMySystem").join("touchpad-disabled.txt")
}

fn write_marker(ids: &[String]) -> std::io::Result<()> {
    let path = marker();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, ids.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn splits_multi_sz() {
        assert_eq!(multi_sz(&wide("a\0bc\0\0")), ["a", "bc"]);
        assert_eq!(multi_sz(&wide("\0\0")), Vec::<String>::new());
        assert_eq!(multi_sz(&wide("tail")), ["tail"]);
    }

    #[test]
    fn matches_only_the_touchpad_collection() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // Touchpad IDs are copied from a real ASUS laptop; the mouse collection uses Windows' standard pattern.
        let touchpad = ids(&[
            "HID\\ASUF1204&Col02",
            "HID\\VID_2808&UP:000D_U:0005",
            "HID_DEVICE_UP:000D_U:0005",
            "HID_DEVICE",
        ]);
        let mouse = ids(&[
            "HID\\ASUF1204&Col01",
            "HID_DEVICE_SYSTEM_MOUSE",
            "HID_DEVICE_UP:0001_U:0002",
            "HID_DEVICE",
        ]);
        assert!(is_touchpad(&touchpad));
        assert!(!is_touchpad(&mouse));
        assert!(!is_touchpad(&[]));
    }
}
