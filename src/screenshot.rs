
pub struct ScreenshotData {
    pub height: usize,
    pub width: usize,
    pub pixels: Vec<u8>,
}

impl ScreenshotData {
    pub fn get_pixel(&self, x: usize, y: usize) -> u32 {
        assert!(x < self.width);
        assert!(y < self.height);

        let pos: usize = y * self.width + x;
        let pos = pos * 4;

        u32::from_ne_bytes([
            self.pixels[pos + 3],
            self.pixels[pos + 2],
            self.pixels[pos + 1],
            self.pixels[pos],
        ])
    }
}

