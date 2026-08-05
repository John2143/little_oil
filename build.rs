//! Windows-only: embed the app icon and version metadata into the exe.
//!
//! `build.rs` runs on the *host*, so the Windows-target check is done via the
//! `CARGO_CFG_TARGET_OS` env var, not `#[cfg]`. Icon embedding fails
//! gracefully (warning, build continues) when `windres` is unavailable, e.g.
//! some cross-compile setups — a real MSVC build on Windows always gets it.

fn main() {
    println!("cargo:rerun-if-changed=assets/little_oil.ico");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }
    let icon = std::path::Path::new("assets/little_oil.ico");
    if !icon.exists() {
        println!("cargo:warning=assets/little_oil.ico missing — building without app icon");
        return;
    }
    match winresource::WindowsResource::new()
        .set_icon("assets/little_oil.ico")
        .set("ProductName", "Little Oil")
        .set("FileDescription", "Little Oil — Path of Exile automation")
        .set("ProductVersion", env!("CARGO_PKG_VERSION"))
        .set("FileVersion", env!("CARGO_PKG_VERSION"))
        .compile()
    {
        Ok(()) => {}
        Err(e) => {
            // windres/rc missing (e.g. mingw cross without binutils) — not fatal.
            println!("cargo:warning=icon embedding skipped: {e}");
        }
    }
}
