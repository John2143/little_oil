use mouse_keyboard_input::{key_codes, Button, VirtualDevice};
use parking_lot::Mutex;
use std::sync::LazyLock;
use tracing::trace;

pub static FAKE_DEVICE: LazyLock<Mutex<VirtualDevice>> =
    LazyLock::new(|| Mutex::new(VirtualDevice::default().unwrap()));

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
    let scale = crate::SETTINGS.read().pointer_scale.unwrap_or(1.25);
    trace!(x, y, scale, "mouse_move");
    pin_cursor_to_origin();
    move_mouse_raw((x as f32 * scale) as i32, (y as f32 * scale) as i32);
}

fn click_release(m: Button) {
    trace!(?m, "click_release");
    let mut device = FAKE_DEVICE.lock();
    device.click(m).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
}

pub fn click(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    click_release(key_codes::BTN_LEFT);
}

pub fn click_right(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    click_release(key_codes::BTN_RIGHT);
}

/// Ctrl + left click — required for PoE inventory/stash item movement.
pub fn ctrl_click(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    let mut device = FAKE_DEVICE.lock();
    device.press(key_codes::KEY_LEFTCTRL).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    device.click(key_codes::BTN_LEFT).unwrap();
    device.release(key_codes::KEY_LEFTCTRL).unwrap();
}
