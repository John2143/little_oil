//! Wayland-native pointer control via the wlr virtual-pointer protocol
//! (`zwlr_virtual_pointer_manager_v1`).
//!
//! Compositors like niri ignore relative EV_REL motion from uinput virtual
//! devices, so the pin-to-origin + relative-move hack used elsewhere in
//! device.rs does not work there (the cursor just snaps to the origin). This
//! module instead speaks the Wayland wire format directly over the socket and
//! uses `motion_absolute`, which repositions the compositor cursor
//! deterministically. The approach mirrors niri-display-settings.

use std::env;
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;

/// Desktop origin and extents of the global output bounding box.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub min_x: i32,
    pub min_y: i32,
    pub width: u32,
    pub height: u32,
}

struct Conn {
    stream: UnixStream,
    pointer: u32,
    bounds: Bounds,
}

enum State {
    Uninit,
    Ready(Conn),
    Failed,
}

static VP: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::Uninit));

const MANAGER_IFACE: &[u8] = b"zwlr_virtual_pointer_manager_v1";

fn now_ms() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0)
}

/// Header for a request: object id, then (size<<16 | opcode).
fn msg(obj: u32, opcode: u16, payload: &[u8]) -> Vec<u8> {
    let size = 8 + payload.len() as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&obj.to_le_bytes());
    out.extend_from_slice(&((size << 16) | opcode as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn wl_string(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&((s.len() as u32) + 1).to_le_bytes());
    out.extend_from_slice(s);
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

fn read_exact(stream: &mut UnixStream, buf: &mut [u8]) -> io::Result<()> {
    let mut read = 0;
    while read < buf.len() {
        let n = stream.read(&mut buf[read..])?;
        if n == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "wayland socket closed",
            ));
        }
        read += n;
    }
    Ok(())
}

fn read_event(stream: &mut UnixStream) -> io::Result<(u32, u16, Vec<u8>)> {
    let mut header = [0u8; 8];
    read_exact(stream, &mut header)?;
    let obj = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let sizeop = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let size = (sizeop >> 16) as usize;
    let opcode = (sizeop & 0xffff) as u16;
    let mut payload = vec![0u8; size - 8];
    read_exact(stream, &mut payload)?;
    Ok((obj, opcode, payload))
}

fn read_u32(payload: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(payload[off..off + 4].try_into().unwrap())
}

/// Read a wayland string at `off`; returns (string, offset past it).
fn read_string(payload: &[u8], off: usize) -> (String, usize) {
    let len = u32::from_le_bytes(payload[off..off + 4].try_into().unwrap()) as usize;
    let start = off + 4;
    let bytes = &payload[start..start + len];
    let s = String::from_utf8_lossy(&bytes[..len - 1]).to_string();
    (s, start + len)
}

fn connect() -> Result<UnixStream> {
    let display = env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    let path: PathBuf = if Path::new(&display).is_absolute() {
        display.into()
    } else {
        let runtime = env::var("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
        Path::new(&runtime).join(&display)
    };
    UnixStream::connect(&path)
        .with_context(|| format!("failed to connect to wayland socket {}", path.display()))
}

/// Global output bounding box from `niri msg outputs`.
///
/// niri's virtual-pointer `motion_absolute` takes coordinates relative to the
/// bounding-box origin with extents equal to the box size, so both the origin
/// and the extents are needed. On any other compositor this bails and
/// device.rs falls back to the uinput path.
fn niri_bounds() -> Result<Bounds> {
    let out = Command::new("niri")
        .args(["msg", "outputs"])
        .output()
        .context("failed to run `niri msg outputs`")?;
    if !out.status.success() {
        bail!(
            "`niri msg outputs` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let mut pos: Option<(i32, i32)> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Logical position:") {
            let mut parts = rest.split(',').map(|s| s.trim().parse::<i32>());
            let (Some(Ok(x)), Some(Ok(y))) = (parts.next(), parts.next()) else {
                continue;
            };
            pos = Some((x, y));
        } else if let Some(rest) = t.strip_prefix("Logical size:") {
            let mut parts = rest.split('x').map(|s| s.trim().parse::<i32>());
            let (Some(Ok(w)), Some(Ok(h))) = (parts.next(), parts.next()) else {
                continue;
            };
            let (x, y) = pos
                .take()
                .context("Logical size seen without a preceding Logical position")?;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + w);
            max_y = max_y.max(y + h);
        }
    }
    if min_x == i32::MAX || max_x == i32::MIN {
        bail!("could not parse output geometry from `niri msg outputs`");
    }
    Ok(Bounds {
        min_x,
        min_y,
        width: (max_x - min_x) as u32,
        height: (max_y - min_y) as u32,
    })
}

impl Conn {
    fn init() -> Result<Conn> {
        let mut stream = connect()?;

        // wl_display.get_registry(2) + wl_display.sync(3).
        let registry = 2u32;
        let cb = 3u32;
        let mut out = Vec::new();
        out.extend_from_slice(&msg(1, 1, &registry.to_le_bytes()));
        out.extend_from_slice(&msg(1, 0, &cb.to_le_bytes()));
        stream.write_all(&out)?;

        let mut manager_name = None;
        loop {
            let (obj, opcode, payload) = read_event(&mut stream)?;
            if obj == 1 && opcode == 0 {
                bail!("wayland error during registry scan");
            }
            if obj == cb && opcode == 0 {
                break;
            }
            if obj == registry && opcode == 0 {
                // wl_registry.global: name, interface, version
                let name = read_u32(&payload, 0);
                let (iface, off) = read_string(&payload, 4);
                let _version = read_u32(&payload, off);
                if iface.as_bytes() == MANAGER_IFACE {
                    manager_name = Some(name);
                }
            }
        }
        let manager_name = manager_name.ok_or_else(|| {
            anyhow::anyhow!("compositor does not expose zwlr_virtual_pointer_manager_v1")
        })?;

        // Bind the manager, create a virtual pointer, sync to flush.
        let manager = 4u32;
        let pointer = 5u32;
        let cb2 = 6u32;
        let mut bind = Vec::new();
        bind.extend_from_slice(&manager_name.to_le_bytes());
        bind.extend_from_slice(&wl_string(MANAGER_IFACE));
        bind.extend_from_slice(&1u32.to_le_bytes());
        bind.extend_from_slice(&manager.to_le_bytes());
        out.clear();
        out.extend_from_slice(&msg(registry, 0, &bind));
        let mut create = Vec::new();
        create.extend_from_slice(&0u32.to_le_bytes()); // seat = null
        create.extend_from_slice(&pointer.to_le_bytes());
        out.extend_from_slice(&msg(manager, 0, &create));
        out.extend_from_slice(&msg(1, 0, &cb2.to_le_bytes()));
        stream.write_all(&out)?;

        loop {
            let (obj, opcode, payload) = read_event(&mut stream)?;
            if obj == cb2 && opcode == 0 {
                break;
            }
            if obj == 1 && opcode == 0 {
                // wl_display.error
                let error_obj = read_u32(&payload, 0);
                let code = read_u32(&payload, 4);
                let (message, _) = read_string(&payload, 8);
                bail!("wayland error on object {error_obj} code {code}: {message}");
            }
        }

        let bounds = niri_bounds()?;
        Ok(Conn {
            stream,
            pointer,
            bounds,
        })
    }

    fn send(&mut self, requests: &[u8]) -> io::Result<()> {
        self.stream.write_all(requests)?;
        self.drain();
        Ok(())
    }

    /// Drop any queued events (registry updates, sync callbacks) so the socket
    /// never fills over a long automation run.
    fn drain(&mut self) {
        if self.stream.set_nonblocking(true).is_err() {
            return;
        }
        let mut buf = [0u8; 4096];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => break,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let _ = self.stream.set_nonblocking(false);
    }

    /// Warp the cursor to desktop coordinates (x, y), converting into the
    /// bounding-box-local space `motion_absolute` expects.
    ///
    /// niri treats the `x`/`y` args as plain coordinates divided by
    /// `x_extent`/`y_extent` (see its `VirtualPointerMotionAbsoluteEvent::x`),
    /// so a plain pixel value with extents equal to the bounding box size maps
    /// 1:1 onto the desktop. wlroots-based compositors expect fixed-point
    /// (value << 8) here instead; this path is intentionally gated to niri via
    /// `niri_bounds`, and non-niri compositors fall back to the uinput path.
    fn move_abs(&mut self, x: i32, y: i32) -> io::Result<()> {
        let (lx, ly) = (x - self.bounds.min_x, y - self.bounds.min_y);
        let mut payload = Vec::new();
        payload.extend_from_slice(&now_ms().to_le_bytes());
        payload.extend_from_slice(&(lx as u32).to_le_bytes());
        payload.extend_from_slice(&(ly as u32).to_le_bytes());
        payload.extend_from_slice(&self.bounds.width.to_le_bytes());
        payload.extend_from_slice(&self.bounds.height.to_le_bytes());
        let mut out = Vec::new();
        out.extend_from_slice(&msg(self.pointer, 1, &payload)); // motion_absolute
        out.extend_from_slice(&msg(self.pointer, 4, &[])); //      frame
        self.send(&out)
    }

    fn button(&mut self, button: u32, pressed: bool) -> io::Result<()> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&now_ms().to_le_bytes());
        payload.extend_from_slice(&button.to_le_bytes());
        payload.extend_from_slice(&(pressed as u32).to_le_bytes());
        let mut out = Vec::new();
        out.extend_from_slice(&msg(self.pointer, 2, &payload)); // button
        out.extend_from_slice(&msg(self.pointer, 4, &[])); //      frame
        self.send(&out)
    }
}

/// Run `f` against the shared virtual pointer, initializing it on first use.
fn with_conn<F>(f: F) -> Result<()>
where
    F: FnOnce(&mut Conn) -> io::Result<()>,
{
    let mut state = VP.lock();
    match &mut *state {
        State::Ready(conn) => f(conn).map_err(anyhow::Error::from),
        State::Failed => bail!("virtual pointer unavailable (init failed earlier)"),
        State::Uninit => {
            let mut conn = match Conn::init() {
                Ok(c) => c,
                Err(e) => {
                    *state = State::Failed;
                    return Err(e);
                }
            };
            let res = f(&mut conn).map_err(anyhow::Error::from);
            *state = State::Ready(conn);
            res
        }
    }
}

/// Warp the cursor to an absolute desktop coordinate.
pub fn move_abs(x: i32, y: i32) -> Result<()> {
    with_conn(|c| c.move_abs(x, y))
}

/// Press or release a wl_pointer button (BTN_LEFT = 0x110, BTN_RIGHT = 0x111).
pub fn button(button: u32, pressed: bool) -> Result<()> {
    with_conn(|c| c.button(button, pressed))
}
