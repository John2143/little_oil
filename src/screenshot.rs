//! Screenshot geometry (Rect, ScreenshotData, pixel access, screen↔frame
//! conversion) and diff-cluster detection.
pub struct ScreenshotData {
    pub height: usize,
    pub width: usize,
    pub pixels: Vec<u8>,
    /// Absolute screen coordinate of pixel (0, 0) — the capture region origin.
    pub origin: (u32, u32),
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

    /// Bounds-checked get_pixel. Returns None outside the frame.
    pub fn try_get_pixel(&self, x: usize, y: usize) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.get_pixel(x, y))
    }

    /// Absolute screen coordinate of a frame pixel. Use for click targets.
    pub fn frame_to_screen(&self, x: u32, y: u32) -> (i32, i32) {
        ((self.origin.0 + x) as i32, (self.origin.1 + y) as i32)
    }

    /// Frame pixel for an absolute screen coordinate. None if outside the frame.
    pub fn screen_to_frame(&self, sx: u32, sy: u32) -> Option<(u32, u32)> {
        let (ox, oy) = self.origin;
        let (dx, dy) = (sx.checked_sub(ox)?, sy.checked_sub(oy)?);
        if dx as usize >= self.width || dy as usize >= self.height {
            return None;
        }
        Some((dx, dy))
    }
}

/// Axis-aligned rectangle in frame-pixel space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn center(&self) -> (u32, u32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }
}

/// Bounding boxes of connected clusters of pixels that differ between `base`
/// and `other`, restricted to `bounds`, discarding clusters smaller than
/// `min_pixels`. 4-connectivity, two-pass labelling with union-find.
pub fn diff_clusters(
    base: &ScreenshotData,
    other: &ScreenshotData,
    bounds: Rect,
    min_pixels: u32,
) -> anyhow::Result<Vec<Rect>> {
    use anyhow::bail;
    use std::collections::HashMap;

    if base.width != other.width || base.height != other.height {
        bail!(
            "base and other dimensions differ: {}x{} vs {}x{}",
            base.width,
            base.height,
            other.width,
            other.height
        );
    }
    if base.origin != other.origin {
        bail!(
            "base and other origins differ: {:?} vs {:?}",
            base.origin,
            other.origin
        );
    }

    // Clamp bounds to the frame.
    let x0 = bounds.x.min(base.width as u32);
    let y0 = bounds.y.min(base.height as u32);
    let x1 = (bounds.x + bounds.width).min(base.width as u32);
    let y1 = (bounds.y + bounds.height).min(base.height as u32);
    if x1 <= x0 || y1 <= y0 {
        return Ok(Vec::new());
    }
    let bw = (x1 - x0) as usize;
    let bh = (y1 - y0) as usize;
    let total = bw * bh;
    if total == 0 {
        return Ok(Vec::new());
    }

    // Pass one: label changed pixels.
    let mut labels: Vec<u32> = vec![0u32; total];
    let mut parent: Vec<u32> = Vec::new(); // union-find; index = label, value = parent
    parent.push(0); // label 0 is unused

    let mut next_label: u32 = 1;

    for by in 0..bh {
        for bx in 0..bw {
            let sx = x0 as usize + bx;
            let sy = y0 as usize + by;
            if base.get_pixel(sx, sy) == other.get_pixel(sx, sy) {
                continue;
            }

            let idx = by * bw + bx;
            let left = if bx > 0 { labels[idx - 1] } else { 0 };
            let up = if by > 0 { labels[idx - bw] } else { 0 };

            let label = if left != 0 && up != 0 {
                if left == up {
                    left
                } else {
                    // Union the two labels.
                    let root = union(&mut parent, left, up);
                    labels[idx] = root;
                    continue;
                }
            } else if left != 0 {
                left
            } else if up != 0 {
                up
            } else {
                let l = next_label;
                next_label += 1;
                parent.push(l);
                l
            };
            labels[idx] = label;
        }
    }

    // Pass two: accumulate bounding boxes and pixel counts per root.
    let mut clusters: HashMap<u32, (u32, u32, u32, u32, u32)> = HashMap::new(); // root -> (min_x, min_y, max_x, max_y, count)
    for by in 0..bh {
        for bx in 0..bw {
            let idx = by * bw + bx;
            let label = labels[idx];
            if label == 0 {
                continue;
            }
            let root = find(&mut parent, label);
            let entry = clusters
                .entry(root)
                .or_insert((bx as u32, by as u32, bx as u32, by as u32, 0));
            let (min_x, min_y, max_x, max_y, count) = entry;
            let bxu = bx as u32;
            let byu = by as u32;
            if bxu < *min_x {
                *min_x = bxu;
            }
            if byu < *min_y {
                *min_y = byu;
            }
            if bxu > *max_x {
                *max_x = bxu;
            }
            if byu > *max_y {
                *max_y = byu;
            }
            *count += 1;
        }
    }

    // Filter by min_pixels and convert to Rect.
    let mut rects: Vec<Rect> = clusters
        .into_iter()
        .filter(|(_, (_, _, _, _, count))| *count >= min_pixels)
        .map(|(_, (min_x, min_y, max_x, max_y, _))| Rect {
            x: x0 + min_x,
            y: y0 + min_y,
            width: max_x - min_x + 1,
            height: max_y - min_y + 1,
        })
        .collect();

    rects.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));
    Ok(rects)
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        parent[x as usize] = parent[parent[x as usize] as usize];
        x = parent[x as usize];
    }
    x
}

fn union(parent: &mut [u32], a: u32, b: u32) -> u32 {
    let ra = find(parent, a);
    let rb = find(parent, b);
    if ra != rb {
        parent[rb as usize] = ra;
    }
    ra
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(w: usize, h: usize, ox: u32, oy: u32, fill: u32) -> ScreenshotData {
        let mut pixels = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let pos = (y * w + x) * 4;
                let bytes = fill.to_ne_bytes();
                pixels[pos] = bytes[3];
                pixels[pos + 1] = bytes[2];
                pixels[pos + 2] = bytes[1];
                pixels[pos + 3] = bytes[0];
            }
        }
        ScreenshotData {
            height: h,
            width: w,
            pixels,
            origin: (ox, oy),
        }
    }

    #[test]
    fn test_diff_clusters_bounding_box_and_min_pixels() {
        let base = make_frame(40, 40, 0, 0, 0xFF000000);
        let mut other = make_frame(40, 40, 0, 0, 0xFF000000);

        // Paint an 8x8 block of a different color at (10, 10).
        // fill 0xFF000000 → bytes [B=0xFF, G=0x00, R=0x00, A=0x00]
        // Changing pos+2 (G) from 0x00 to 0xFF makes the pixel differ.
        for y in 10..18 {
            for x in 10..18 {
                let pos = (y * 40 + x) * 4;
                other.pixels[pos + 2] = 0xFF;
            }
        }
        // Stray single pixel at (30, 30).
        {
            let pos = (30 * 40 + 30) * 4;
            other.pixels[pos + 2] = 0xFF;
        }

        let bounds = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 40,
        };
        let clusters = diff_clusters(&base, &other, bounds, 20).unwrap();
        assert_eq!(
            clusters.len(),
            1,
            "expected exactly 1 cluster, got: {:?}",
            clusters
        );
        assert_eq!(
            clusters[0],
            Rect {
                x: 10,
                y: 10,
                width: 8,
                height: 8,
            }
        );
    }

    /// Paint a U shape. Its two arms get separate labels during the raster
    /// scan and only merge on the bottom bar, which is the ONLY path that
    /// exercises the union-find branch in pass one. Without this the solid
    /// rectangle test above never touches that code.
    #[test]
    fn test_diff_clusters_merges_late_joining_arms() {
        let base = make_frame(40, 40, 0, 0, 0xFF000000);
        let mut other = make_frame(40, 40, 0, 0, 0xFF000000);

        let mut paint = |x: usize, y: usize| {
            let pos = (y * 40 + x) * 4;
            other.pixels[pos + 2] = 0xFF;
        };

        // Left arm and right arm, 6 rows tall, 4 px wide, 12 px apart.
        for y in 5..11 {
            for x in 5..9 {
                paint(x, y);
            }
            for x in 17..21 {
                paint(x, y);
            }
        }
        // Bottom bar joins them at row 11..13.
        for y in 11..13 {
            for x in 5..21 {
                paint(x, y);
            }
        }

        let bounds = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 40,
        };
        let clusters = diff_clusters(&base, &other, bounds, 4).unwrap();
        assert_eq!(
            clusters.len(),
            1,
            "U shape must merge into ONE cluster; got {:?}",
            clusters
        );
        // Spans x 5..=20 and y 5..=12.
        assert_eq!(
            clusters[0],
            Rect {
                x: 5,
                y: 5,
                width: 16,
                height: 8
            }
        );
    }

    #[test]
    fn test_diff_clusters_dimension_mismatch() {
        let base = make_frame(40, 40, 0, 0, 0);
        let other = make_frame(41, 40, 0, 0, 0);
        let bounds = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 40,
        };
        assert!(diff_clusters(&base, &other, bounds, 1).is_err());
    }

    #[test]
    fn test_diff_clusters_origin_mismatch() {
        let base = make_frame(40, 40, 0, 0, 0);
        let other = make_frame(40, 40, 10, 0, 0);
        let bounds = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 40,
        };
        assert!(diff_clusters(&base, &other, bounds, 1).is_err());
    }

    #[test]
    fn test_offset_conversions() {
        let frame = ScreenshotData {
            height: 300,
            width: 400,
            pixels: vec![0u8; 400 * 300 * 4],
            origin: (2560, 100),
        };

        // frame_to_screen
        assert_eq!(frame.frame_to_screen(10, 20), (2570, 120));

        // screen_to_frame — inside
        assert_eq!(frame.screen_to_frame(2570, 120), Some((10, 20)));

        // screen_to_frame — left of origin (exercises checked_sub)
        assert_eq!(frame.screen_to_frame(2000, 120), None);

        // screen_to_frame — right of frame
        assert_eq!(frame.screen_to_frame(3000, 120), None);

        // screen_to_frame — below frame
        assert_eq!(frame.screen_to_frame(2570, 500), None);
    }
}
