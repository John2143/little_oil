//! Windows backend: WGC (Windows.Graphics.Capture) window capture, win32
//! clipboard, window discovery, and DPI setup.
//!
//! Capture design:
//! * `screenshot` captures the PoE game window with Windows.Graphics.Capture.
//!   BitBlt is *not* used for the game window — DX11 content reads back black
//!   through GDI. WGC reads the DWM-composited surface directly.
//! * The capture is synchronous: a capture session is started on demand, the
//!   first frame is awaited (WGC delivers it after ~1-2 frames), pixels are
//!   read back from the frame surface, and the session is torn down. This
//!   matches the one-shot screenshot model the macros assume.
//!
//! `frame_to_screen` continues to map capture pixels to click coordinates.

use crate::ScreenRegion;
use crate::Settings;
use crate::screenshot::ScreenshotData;
use anyhow::{Context, Result, bail};
use std::time::{Duration, Instant};
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::UI::WindowId;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_9_3, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::{DXGI_MAP_READ, DXGI_MAPPED_RECT, IDXGIDevice, IDXGISurface};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::core::{BOOL, IInspectable, Interface};

/// Set DPI awareness so capture and input coordinates are physical pixels.
/// Call once at startup (main.rs). Fails silently if already set by the host.
pub fn set_dpi_awareness() {
    unsafe {
        use windows::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
        };
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn window_title(hwnd: HWND) -> String {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let got = GetWindowTextW(hwnd, &mut buf);
        if got > 0 {
            String::from_utf16_lossy(&buf[..got as usize])
        } else {
            String::new()
        }
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, IsWindowVisible};
        let slot = &mut *(lparam.0 as *mut Option<HWND>);
        let mut class = [0u16; 128];
        let len = GetClassNameW(hwnd, &mut class);
        let class_name = if len > 0 {
            String::from_utf16_lossy(&class[..len as usize])
        } else {
            String::new()
        };
        let title = window_title(hwnd);
        let known_classes = ["POEWindowClass", "PathOfExile"];
        let matches = known_classes
            .iter()
            .any(|c| class_name.eq_ignore_ascii_case(c))
            || title.to_lowercase().contains("path of exile");
        if matches && IsWindowVisible(hwnd).as_bool() {
            *slot = Some(hwnd);
            return BOOL(0); // stop enumeration
        }
        BOOL(1)
    }
}

/// Find the Path of Exile window handle (title or known window class).
pub fn find_game_window() -> Option<HWND> {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
        let mut found: Option<HWND> = None;
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut found as *mut _ as isize));
        found
    }
}

/// Client area size and its top-left screen position (physical pixels).
fn client_rect(hwnd: HWND) -> Result<(u32, u32, i32, i32)> {
    unsafe {
        use windows::Win32::Graphics::Gdi::ClientToScreen;
        use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect as *mut _).context("GetClientRect failed")?;
        let w = (rect.right - rect.left) as u32;
        let h = (rect.bottom - rect.top) as u32;
        let mut pt = POINT::default();
        if ClientToScreen(hwnd, &mut pt as *mut _) == BOOL(0) {
            bail!("ClientToScreen failed for game window");
        }
        Ok((w, h, pt.x, pt.y))
    }
}

/// D3D11 device + WGC interop device, created once and reused.
struct CaptureDevice {
    _d3d_device: ID3D11Device,
    wgc_device: IDirect3DDevice,
}

fn capture_device() -> Result<CaptureDevice> {
    unsafe {
        // CoInitialize on this thread; RPC_E_CHANGED_MODE (already init) is fine.
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        if !hr.is_ok() && hr.0 != 0x8001_0106u32 as i32 {
            tracing::trace!(?hr, "CoInitializeEx returned unexpected hr");
        }
        let mut device: Option<ID3D11Device> = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_0,
                D3D_FEATURE_LEVEL_9_3,
            ]),
            7,
            Some(&mut device as *mut Option<ID3D11Device>),
            None,
            None,
        )
        .context("D3D11CreateDevice failed — is a GPU driver installed?")?;
        let device =
            device.ok_or_else(|| anyhow::anyhow!("D3D11CreateDevice returned no device"))?;

        let dxgi: IDXGIDevice = device
            .cast()
            .context("ID3D11Device is not an IDXGIDevice")?;
        let inspectable: IInspectable = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
            .context("CreateDirect3D11DeviceFromDXGIDevice failed")?;
        let wgc_device: IDirect3DDevice = inspectable
            .cast()
            .context("device is not an IDirect3DDevice")?;
        Ok(CaptureDevice {
            _d3d_device: device,
            wgc_device,
        })
    }
}

/// The GraphicsCaptureItem for a window. For desktop apps, WindowId is the
/// HWND value, which is the documented equivalence.
fn capture_item_for_window(hwnd: HWND) -> Result<GraphicsCaptureItem> {
    GraphicsCaptureItem::TryCreateFromWindowId(WindowId {
        Value: hwnd.0 as u64,
    })
    .context("TryCreateFromWindowId failed — is the game window visible and not minimized?")
}

/// Capture one frame of the game window synchronously.
pub fn screenshot(settings: &Settings) -> Result<ScreenshotData> {
    let _ = settings;
    let dev = capture_device()?;
    let hwnd = find_game_window().ok_or_else(|| {
        anyhow::anyhow!("Could not find a Path of Exile window — is the game open?")
    })?;
    let (cw, ch, ox, oy) = client_rect(hwnd)?;
    if cw == 0 || ch == 0 {
        bail!("game window has zero client size — is it minimized?");
    }

    let item = capture_item_for_window(hwnd)?;

    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &dev.wgc_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        SizeInt32 {
            Width: cw as i32,
            Height: ch as i32,
        },
    )
    .context("Direct3D11CaptureFramePool::CreateFreeThreaded failed")?;

    let session: GraphicsCaptureSession = pool
        .CreateCaptureSession(&item)
        .context("CreateCaptureSession failed")?;
    session.StartCapture().context("StartCapture failed")?;

    // First frame typically arrives after 1-2 compositor frames.
    let mut frame = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(f) = pool.TryGetNextFrame() {
            frame = Some(f);
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let frame = frame.ok_or_else(|| anyhow::anyhow!("timed out waiting for WGC frame"))?;

    // Read pixels: the frame surface is an IDXGISurface we can Map directly.
    let surface: IDXGISurface = frame
        .Surface()?
        .cast()
        .context("frame surface is not an IDXGISurface")?;
    let mut mapped = DXGI_MAPPED_RECT::default();
    unsafe {
        surface
            .Map(&mut mapped, DXGI_MAP_READ)
            .context("IDXGISurface::Map failed")?;
    }

    // DXGI_FORMAT_B8G8R8A8_UNORM → ScreenshotData stores RGBA8, swap R/B.
    let pitch = mapped.Pitch as usize;
    let w = cw as usize;
    let h = ch as usize;
    let mut pixels = vec![0u8; w * h * 4];
    unsafe {
        for y in 0..h {
            let row_src = (mapped.pBits as *const u8).add(y * pitch);
            let row_dst = &mut pixels[y * w * 4..(y + 1) * w * 4];
            for (i, chunk) in row_dst.chunks_exact_mut(4).enumerate() {
                let b = *row_src.add(i * 4);
                let g = *row_src.add(i * 4 + 1);
                let r = *row_src.add(i * 4 + 2);
                let a = *row_src.add(i * 4 + 3);
                chunk.copy_from_slice(&[r, g, b, a]);
            }
        }
        let _ = surface.Unmap();
    }

    // Tear down the session; dropping pool+session stops capture.
    drop(session);
    drop(pool);
    drop(item);
    drop(frame);

    Ok(ScreenshotData {
        width: w,
        height: h,
        pixels,
        origin: (ox, oy),
    })
}

/// Full virtual-desktop capture (all monitors composited), no cursor. Used by
/// the GUI calibration preview. BitBlt of the desktop DC reads the DWM
/// composited screen; in exclusive-fullscreen games the game surface may read
/// black — borderless/windowed modes (the PoE default) capture cleanly.
pub fn capture_desktop() -> Result<ScreenshotData> {
    unsafe {
        use windows::Win32::Graphics::Gdi::{
            BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY,
            SelectObject,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        };
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if w <= 0 || h <= 0 {
            bail!("virtual screen size is {w}x{h} — cannot capture");
        }

        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(None);
        let bmp = CreateCompatibleBitmap(screen_dc, w, h);
        if screen_dc.is_invalid() || mem_dc.is_invalid() || bmp.is_invalid() {
            bail!("failed to create desktop capture surfaces");
        }

        let old = SelectObject(mem_dc, bmp.into());
        // The screen DC is anchored at the primary monitor's (0,0); the virtual
        // screen may extend left/up, so offset the source by (-vx, -vy).
        BitBlt(mem_dc, 0, 0, w, h, Some(screen_dc), -vx, -vy, SRCCOPY).context("BitBlt failed")?;

        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0, // BI_RGB
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        let got = GetDIBits(
            mem_dc,
            bmp,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        if got == 0 {
            bail!("GetDIBits failed");
        }

        // 32bpp BI_RGB is BGRA little-endian; ScreenshotData is RGBA8.
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
        }

        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(None, screen_dc);

        Ok(ScreenshotData {
            width: w as usize,
            height: h as usize,
            pixels,
            origin: (vx, vy),
        })
    }
}

/// Full-desktop capture with cursor. Not implemented on Windows — only the
/// Linux cursor-diff pointer calibration uses it.
pub fn capture_all() -> Result<ScreenshotData> {
    bail!("capture_all is not implemented on Windows (only used by Linux pointer calibration)")
}

/// Bounds of the window under a screen point (`WindowFromPoint`).
pub fn window_under_point(x: i32, y: i32) -> Option<ScreenRegion> {
    unsafe {
        use windows::Win32::Foundation::{POINT, RECT};
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, WindowFromPoint};
        let hwnd = WindowFromPoint(POINT { x, y });
        if hwnd.is_invalid() {
            return None;
        }
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r as *mut _).is_err() {
            return None;
        }
        clamp_region(r.left, r.top, r.right - r.left, r.bottom - r.top)
    }
}

/// Bounds of the monitor under a screen point (`MonitorFromPoint`).
pub fn monitor_under_point(x: i32, y: i32) -> Option<ScreenRegion> {
    unsafe {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
        };
        let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        if hmon.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut info as *mut _).as_bool() == false {
            return None;
        }
        let r = info.rcMonitor;
        clamp_region(r.left, r.top, r.right - r.left, r.bottom - r.top)
    }
}

/// Clamp window/monitor bounds into the non-negative `ScreenRegion` type.
fn clamp_region(x: i32, y: i32, w: i32, h: i32) -> Option<ScreenRegion> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x + w).max(0) as u32;
    let y1 = (y + h).max(0) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(ScreenRegion {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

pub fn select_region(prompt: &str) -> Result<ScreenRegion> {
    // Region selection is interactive; `little_oil gui` provides drag-to-select
    // on a screenshot preview (Phase 3). The CLI path explains that.
    bail!("{prompt}\nRegion selection on Windows is done from the GUI: run `little_oil gui`")
}

// ── clipboard ─────────────────────────────────────────────────────

/// Clear the system clipboard.
pub fn clear_clipboard() -> Result<()> {
    unsafe {
        use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard};
        OpenClipboard(None).context("OpenClipboard failed — clipboard held by another process?")?;
        let _ = EmptyClipboard();
        let _ = CloseClipboard();
    }
    Ok(())
}

/// Read UTF-16 text from the clipboard, if present.
pub fn read_clipboard_text() -> Option<String> {
    // CF_UNICODETEXT == 13
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        use windows::Win32::System::DataExchange::{
            CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
        };
        use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return None;
        }
        OpenClipboard(None).ok()?;
        let result = (|| {
            let handle: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
            let hglobal = HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                return None;
            }
            let size = GlobalSize(hglobal);
            let bytes = std::slice::from_raw_parts(ptr as *const u8, size);
            let words: &[u16] = &bytes.align_to::<u16>().1;
            let words: Vec<u16> = words.iter().take_while(|w| **w != 0).copied().collect();
            let _ = GlobalUnlock(hglobal);
            Some(String::from_utf16_lossy(&words))
        })();
        let _ = CloseClipboard();
        result
    }
}
