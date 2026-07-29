use mouse_keyboard_input::{key_codes, Button, VirtualDevice};
use parking_lot::Mutex;
use std::sync::LazyLock;
use tracing::trace;

pub static FAKE_DEVICE: LazyLock<Mutex<VirtualDevice>> =
    LazyLock::new(|| Mutex::new(VirtualDevice::default().unwrap()));

pub fn move_mouse(x: i32, y: i32) {
    trace!(x, y, "mouse_move");
    let mut device = FAKE_DEVICE.lock();
    device.move_mouse(-5000, -5000).unwrap();
    device.move_mouse((x as f32 * 1.25) as _, (y as f32 * 1.25) as _).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
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
