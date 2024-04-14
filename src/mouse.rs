
#[cfg(feature = "input_wayland")]
pub use wayland::*;
#[cfg(all(not(feature = "input_wayland"), feature = "input_x"))]
pub use x::*;

#[cfg(feature = "input_x")]
pub(super) mod x {
    use tracing::trace;
    use once_cell::sync::Lazy;
    use mouse_rs::types::keys::Keys;

    thread_local! {
        static FAKE_DEVICE: Lazy<mouse_rs::Mouse> = Lazy::new(|| {
            trace!("Generating new mouse instance for thread");
            mouse_rs::Mouse::new()
        });
    }

    pub fn init() {
        FAKE_DEVICE.with(|mouse| trace!(mouse_pos = ?mouse.get_position().unwrap(), "Warming up the mouse on this thread"))
    }

    pub fn click(x: i32, y: i32) {
        move_mouse(x, y);
        std::thread::sleep(std::time::Duration::from_millis(30));
        click_release(&Keys::RIGHT);
    }

    pub fn click_right(x: i32, y: i32) {
        move_mouse(x, y);
        std::thread::sleep(std::time::Duration::from_millis(30));
        click_release(&Keys::LEFT);
    }

    pub fn click_release(key: &Keys) {
        FAKE_DEVICE.with(|mouse| {
            mouse.press(key);
        });
        std::thread::sleep(std::time::Duration::from_millis(10));

        FAKE_DEVICE.with(|mouse| {
            mouse.release(key);
        });
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    pub fn move_mouse(x: i32, y: i32) {
        trace!(x, y, "mouse_move");
        FAKE_DEVICE.with(|mouse| {
            mouse.move_to(x, y)
        });

        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(feature = "input_wayland")]
pub(super) mod wayland {
    use mouse_keyboard_input::{key_codes, Button, VirtualDevice};
    use once_cell::sync::Lazy;
    use tracing::trace;

    use std::sync::Mutex;

    pub fn init() {
        FAKE_DEVICE.lock().unwrap().synchronize().unwrap();
    }

    static FAKE_DEVICE: Lazy<Mutex<VirtualDevice>> =
        Lazy::new(|| Mutex::new(VirtualDevice::default().unwrap()));

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

    pub fn click_release(m: Button) {
        trace!(?m, "click_release");
        let mut device = FAKE_DEVICE.lock().unwrap();

        device.click(m).unwrap();
        //device.synchronize().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    pub fn move_mouse(x: i32, y: i32) {
        trace!(x, y, "mouse_move");
        let mut device = FAKE_DEVICE.lock().unwrap();
        device.move_mouse(-5000, -5000).unwrap();
        device
            .move_mouse((x as f32 * 1.25) as _, (y as f32 * 1.25) as _)
            .unwrap();
        //device.synchronize().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

