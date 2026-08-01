use mouse_keyboard_input::{key_codes, Button, VirtualDevice};
use parking_lot::Mutex;
use std::sync::LazyLock;
use tracing::trace;

use crate::platform::{virtual_pointer, Platform};

pub static FAKE_DEVICE: LazyLock<Mutex<VirtualDevice>> =
    LazyLock::new(|| Mutex::new(VirtualDevice::default().unwrap()));

/// True when pointer control should go through the Wayland virtual pointer
/// (absolute positioning) instead of the uinput relative-motion path.
fn wayland_pointer() -> bool {
    let platform = crate::SETTINGS.read().platform;
    matches!(platform, Some(Platform::Wayland)) || (platform.is_none() && !cfg!(windows))
}

/// Emit a raw relative pointer move in device units, with no scaling.
/// Used by pointer calibration; normal callers want move_mouse.
pub fn move_mouse_raw(dx: i32, dy: i32) {
    let mut device = FAKE_DEVICE.lock();
    device.move_mouse(dx, dy).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
}

/// Pin the cursor to the desktop origin (0, 0).
pub fn pin_cursor_to_origin() {
    move_mouse_raw(-50000, -50000);
}

pub fn move_mouse(x: i32, y: i32) {
    if wayland_pointer() {
        if virtual_pointer::move_abs(x, y).is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(10));
            return;
        }
        trace!(x, y, "virtual pointer failed; falling back to uinput");
    }
    let scale = crate::SETTINGS.read().pointer_scale.unwrap_or(1.25);
    trace!(x, y, scale, "mouse_move");
    pin_cursor_to_origin();
    move_mouse_raw((x as f32 * scale) as i32, (y as f32 * scale) as i32);
}

/// Send a mouse button press/release through whichever pointer path is active.
fn emit_button(button: Button, pressed: bool) {
    if wayland_pointer() {
        if virtual_pointer::button(button as u32, pressed).is_ok() {
            return;
        }
    }
    let mut device = FAKE_DEVICE.lock();
    if pressed {
        device.press(button).unwrap();
    } else {
        device.release(button).unwrap();
    }
}

pub fn click(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    emit_button(key_codes::BTN_LEFT, true);
    std::thread::sleep(std::time::Duration::from_millis(10));
    emit_button(key_codes::BTN_LEFT, false);
}

pub fn click_right(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    emit_button(key_codes::BTN_RIGHT, true);
    std::thread::sleep(std::time::Duration::from_millis(10));
    emit_button(key_codes::BTN_RIGHT, false);
}

/// Ctrl + left click — required for PoE inventory/stash item movement.
pub fn ctrl_click(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    let mut device = FAKE_DEVICE.lock();
    device.press(key_codes::KEY_LEFTCTRL).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    drop(device);
    emit_button(key_codes::BTN_LEFT, true);
    std::thread::sleep(std::time::Duration::from_millis(10));
    emit_button(key_codes::BTN_LEFT, false);
    FAKE_DEVICE.lock().release(key_codes::KEY_LEFTCTRL).unwrap();
}

/// Ctrl + right click — moves an item while keeping Ctrl held, used by the
/// `emptyr` command for PoE inventory emptying.
pub fn ctrl_right_click(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    let mut device = FAKE_DEVICE.lock();
    device.press(key_codes::KEY_LEFTCTRL).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    drop(device);
    emit_button(key_codes::BTN_RIGHT, true);
    std::thread::sleep(std::time::Duration::from_millis(10));
    emit_button(key_codes::BTN_RIGHT, false);
    FAKE_DEVICE.lock().release(key_codes::KEY_LEFTCTRL).unwrap();
}
