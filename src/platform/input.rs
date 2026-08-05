//! Cross-platform input injection: mouse move, buttons, and keys.
//!
//! The `Input` enum owns the per-OS backend state and is constructed once in
//! `App::new`. All pointer and keyboard operations in `app.rs` channel through
//! the methods here, keeping the rest of the code platform-agnostic.

use crate::platform::Platform;

/// Mouse button abstraction — platform codes live in the backend, not leaked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputButton {
    Left,
    Right,
}

/// Keyboard keys used by the automation — only the minimal set actually needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputKey {
    Ctrl,
    Alt,
    C,
}

/// Cross-platform input state. The backend is selected at compile time.
///
/// * Linux   — `VirtualDevice` (uinput) with optional `VirtualPointer` (wlr).
/// * Windows — `SendInput` (absolute positioning, no scale needed).
/// * Other   — compile error (unsupported platform).
pub(crate) enum Input {
    #[cfg(target_os = "linux")]
    Linux {
        device: mouse_keyboard_input::VirtualDevice,
        vpointer: crate::platform::virtual_pointer::VirtualPointer,
    },
    #[cfg(target_os = "windows")]
    Windows,
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    Unsupported,
}

impl Input {
    pub(crate) fn new(_platform: Platform) -> anyhow::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            use mouse_keyboard_input::VirtualDevice;
            let mut device = VirtualDevice::default().map_err(|e| {
                anyhow::anyhow!(
                    "failed to open uinput device — check /dev/uinput exists and \
                     your user can write to it: {e}"
                )
            })?;
            device
                .synchronize()
                .map_err(|e| anyhow::anyhow!("failed to synchronize uinput device: {e}"))?;
            Ok(Input::Linux {
                device,
                vpointer: crate::platform::virtual_pointer::VirtualPointer::Uninit,
            })
        }
        #[cfg(target_os = "windows")]
        {
            // No init needed — SendInput is stateless.
            Ok(Input::Windows)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            compile_error!("little_oil only supports Linux and Windows")
        }
    }

    // ── raw primitives ──────────────────────────────────────────

    /// Raw relative pointer move in device units, with no scaling. Used by
    /// pointer calibration on Linux; panics on Windows (absolute positioning
    /// makes calibration unnecessary).
    #[cfg(target_os = "linux")]
    pub(crate) fn move_mouse_raw(&mut self, dx: i32, dy: i32) {
        let Input::Linux { device, .. } = self;
        if let Err(e) = device.move_mouse(dx, dy) {
            tracing::error!(?e, "uinput move_mouse failed");
        }
    }

    /// Move the cursor to an absolute screen coordinate. On Linux this uses the
    /// wlr virtual pointer when available (niri), falling back to the uinput
    /// relative path with `pointer_scale`. On Windows this is a direct
    /// `SetCursorPos` call (no scale).
    pub(crate) fn move_mouse(&mut self, x: i32, y: i32, pointer_scale: f32, platform: Platform) {
        #[cfg(target_os = "linux")]
        if matches!(platform, Platform::Wayland) {
            let Input::Linux { vpointer, .. } = self;
            if vpointer.move_abs(x, y).is_ok() {
                std::thread::sleep(std::time::Duration::from_millis(10));
                return;
            }
            tracing::trace!(x, y, "virtual pointer failed; falling back to uinput");
        }
        #[cfg(target_os = "linux")]
        {
            // uinput relative path: pin to origin, then move relative with scale.
            Self::pin_cursor_to_origin_impl(self);
            let sx = (x as f32 * pointer_scale) as i32;
            let sy = (y as f32 * pointer_scale) as i32;
            self.move_mouse_raw(sx, sy);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        #[cfg(target_os = "windows")]
        {
            // SetCursorPos takes physical-pixel screen coordinates directly.
            // SAFETY: SetCursorPos is always available on Windows.
            unsafe {
                extern "system" {
                    fn SetCursorPos(x: i32, y: i32) -> i32;
                }
                SetCursorPos(x, y);
            }
            // A tiny settle beat mirrors the Linux 10ms post-move sleep.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Pin the cursor to the desktop origin. Linux only: uinput relative path
    /// needs this before each absolute move. Windows is always absolute.
    #[cfg(target_os = "linux")]
    fn pin_cursor_to_origin_impl(this: &mut Input) {
        let Input::Linux { device, .. } = this;
        if let Err(e) = device.move_mouse(-50000, -50000) {
            tracing::error!(?e, "uinput pin-to-origin failed");
        }
    }

    // ── buttons ─────────────────────────────────────────────────

    /// Press or release a mouse button on the current backend.
    pub(crate) fn button(&mut self, btn: InputButton, pressed: bool, platform: Platform) {
        #[cfg(target_os = "linux")]
        if matches!(platform, Platform::Wayland) {
            let Input::Linux { vpointer, .. } = self;
            let code = match btn {
                InputButton::Left => 0x110u32,
                InputButton::Right => 0x111u32,
            };
            if vpointer.button(code, pressed).is_ok() {
                return;
            }
        }
        #[cfg(target_os = "linux")]
        {
            use mouse_keyboard_input::key_codes;
            let evdev = match btn {
                InputButton::Left => key_codes::BTN_LEFT,
                InputButton::Right => key_codes::BTN_RIGHT,
            };
            let Input::Linux { device, .. } = self;
            if pressed {
                if let Err(e) = device.press(evdev) {
                    tracing::error!(?e, "uinput press failed");
                }
            } else if let Err(e) = device.release(evdev) {
                tracing::error!(?e, "uinput release failed");
            }
        }
        #[cfg(target_os = "windows")]
        unsafe {
            extern "system" {
                fn SendInput(c_inputs: u32, p_inputs: *const u8, cb_size: i32) -> u32;
            }
            #[repr(C)]
            struct MouseInput {
                dx: i32,
                dy: i32,
                mouse_data: u32,
                dw_flags: u32,
                time: u32,
                dw_extra_info: usize,
            }
            #[repr(C)]
            struct Input {
                type_: u32,
                mi: MouseInput,
            }
            let flags = match (btn, pressed) {
                (InputButton::Left, true) => 0x0002u32,
                (InputButton::Left, false) => 0x0004u32,
                (InputButton::Right, true) => 0x0008u32,
                (InputButton::Right, false) => 0x0010u32,
            };
            let inp = Input {
                type_: 0, // INPUT_MOUSE
                mi: MouseInput {
                    dx: 0,
                    dy: 0,
                    mouse_data: 0,
                    dw_flags: flags,
                    time: 0,
                    dw_extra_info: 0,
                },
            };
            SendInput(
                1,
                &inp as *const _ as *const u8,
                std::mem::size_of::<Input>() as i32,
            );
        }
    }

    // ── keyboard ────────────────────────────────────────────────

    /// Press or release a keyboard key (minimal set: Ctrl, Alt, C).
    pub(crate) fn key(&mut self, key: InputKey, pressed: bool) {
        #[cfg(target_os = "linux")]
        {
            use mouse_keyboard_input::key_codes;
            let code = match key {
                InputKey::Ctrl => key_codes::KEY_LEFTCTRL,
                InputKey::Alt => key_codes::KEY_LEFTALT,
                InputKey::C => key_codes::KEY_C,
            };
            let Input::Linux { device, .. } = self;
            if pressed {
                if let Err(e) = device.press(code) {
                    tracing::error!(?e, "uinput key press failed ({key:?})");
                }
            } else if let Err(e) = device.release(code) {
                tracing::error!(?e, "uinput key release failed ({key:?})");
            }
        }
        #[cfg(target_os = "windows")]
        unsafe {
            extern "system" {
                fn SendInput(c_inputs: u32, p_inputs: *const u8, cb_size: i32) -> u32;
                fn MapVirtualKeyW(u_code: u32, u_map_type: u32) -> u32;
            }
            #[repr(C)]
            struct KeyboardInput {
                w_vk: u16,
                w_scan: u16,
                dw_flags: u32,
                time: u32,
                dw_extra_info: usize,
            }
            #[repr(C)]
            struct Input {
                type_: u32,
                ki: KeyboardInput,
            }
            let vk = match key {
                InputKey::Ctrl => 0x11u32,
                InputKey::Alt => 0x12u32,
                InputKey::C => 0x43u32,
            };
            let scan = MapVirtualKeyW(vk, 0) as u16; // MAPVK_VK_TO_VSC = 0
            let flags = (scan as u32) << 16 | if pressed { 0x0000 } else { 0x0002 };
            let inp = Input {
                type_: 1, // INPUT_KEYBOARD
                ki: KeyboardInput {
                    w_vk: 0,
                    w_scan: scan,
                    dw_flags: flags,
                    time: 0,
                    dw_extra_info: 0,
                },
            };
            SendInput(
                1,
                &inp as *const _ as *const u8,
                std::mem::size_of::<Input>() as i32,
            );
        }
    }
}
