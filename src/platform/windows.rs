use crate::screenshot::ScreenshotData;
use crate::ScreenRegion;
use crate::Settings;
use anyhow::bail;

pub fn screenshot(_settings: &Settings) -> anyhow::Result<ScreenshotData> {
    bail!("Windows screenshot not implemented")
}

pub fn select_region(_prompt: &str) -> anyhow::Result<ScreenRegion> {
    bail!("Windows region selection not implemented")
}
