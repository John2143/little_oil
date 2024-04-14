use anyhow::bail;
use tracing::{debug, info, trace};

use crate::Settings;

/// This is the data returned from a screenshot
pub struct ScreenshotData {
    height: usize,
    width: usize,
    pixels: Vec<u8>,
}

#[cfg(feature = "input_wayland")]
pub fn take_screenshot_grim(settings: &Settings) -> anyhow::Result<ScreenshotData> {
    let wloc = settings.poe_window_location;
    let cmd = Command::new("grim")
        // whole left screen
        .arg("-g")
        .arg(format!(
            "{x},{y} {w}x{h}",
            x = wloc.x,
            y = wloc.y,
            w = wloc.width,
            h = wloc.height
        ))
        // png out
        .arg("-t")
        .arg("ppm")
        .arg("-")
        .output()
        .unwrap();

    // for .seek()
    let stdout = Cursor::new(cmd.stdout);
    // the output format ppm "portable pixel map" from grim is called
    // pnm "portable any map" in the image crate.
    let img = image::load(stdout, image::ImageFormat::Pnm)
        .context("Failed to load screenshot from output of grim.")?;

    //let path = Path::new("./last_screnshot.png");
    //info!(path = ?path.canonicalize().unwrap(), "saving screenshot");
    //img.save(path).unwrap();

    Ok(ScreenshotData {
        height: img.height() as usize,
        width: img.width() as usize,
        pixels: img.to_rgba8().to_vec(),
    })
}

#[cfg(feature = "input_x")]
pub fn take_screenshot_scrap(
    settings: &Settings,
    monitor_index: usize,
) -> anyhow::Result<ScreenshotData> {
    use anyhow::Context;

    trace!(?settings, "taking screenshot...");

    // Get a handle to the display throgh scrap
    let disps = scrap::Display::all()?;
    let disp = disps.into_iter().nth(monitor_index).with_context(|| {
        format!("monitor index of {monitor_index} is out of bounds for this GPU.")
    })?;
    let mut cap = scrap::Capturer::new(disp).unwrap();
    let width = cap.width();
    let height = cap.height();

    info!(
        width,
        height,
        monitor_index,
        "Taking a screenshot on this monitor"
    );

    let sleep = 50;

    //max 2 seconds before fail
    let maxloops = 2000 / sleep;

    debug!("trying to screenshot...");

    for _ in 0..maxloops {
        match cap.frame() {
            Ok(fr) => {
                trace!("got screenshot");
                return Ok(ScreenshotData {
                    height,
                    width,
                    pixels: fr.to_vec(),
                });
            }
            Err(e) => {
                trace!(?e, "screenshot failed.");
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(sleep));
    }

    bail!("was not able to take screenshot after {maxloops} tries");
}

impl ScreenshotData {
    //return RGBA8888 pixel as u32
    pub fn get_pixel(&self, x: usize, y: usize) -> u32 {
        assert!(x < self.width);
        assert!(y < self.height);

        let pos: usize = y * self.width + x;
        let pos = pos * 4; //pixel format ARGB8888;

        //TODO find the rust idiomatic way to do this
        unsafe {
            std::mem::transmute([
                self.pixels[pos + 3],
                self.pixels[pos + 2],
                self.pixels[pos + 1],
                self.pixels[pos],
            ])
        }
    }
}
