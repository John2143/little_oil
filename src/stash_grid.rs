use serde::{Deserialize, Serialize};
use crate::screenshot::{Rect, ScreenshotData};

/// Quad tab is 24x24.
pub const QUAD_COLS: usize = 24;
pub const QUAD_ROWS: usize = 24;

/// Calibrated quad-tab geometry, in frame-pixel space (relative to the game
/// window region). Convert to screen with ScreenshotData::to_screen.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StashGrid {
    /// Left edge of each column. Exactly QUAD_COLS entries.
    pub cols: [u32; QUAD_COLS],
    /// Top edge of each row. Exactly QUAD_ROWS entries.
    pub rows: [u32; QUAD_ROWS],
    pub cell_w: u32,
    pub cell_h: u32,
    /// Search-highlight border color, as packed by ScreenshotData::get_pixel.
    pub highlight_color: u32,
}

impl StashGrid {
    /// Middle of cell (col, row), in frame-pixel space.
    pub fn cell_center(&self, col: usize, row: usize) -> (u32, u32) {
        (self.cols[col] + self.cell_w / 2, self.rows[row] + self.cell_h / 2)
    }

    /// The three detection probes: the cell's bottom boundary row at 25%, 50%
    /// and 75% of its width. The highlight border is drawn on the boundary,
    /// and item art can bleed past it, so the interior is not reliable.
    pub fn probes(&self, col: usize, row: usize) -> [(usize, usize); 3] {
        let y = (self.rows[row] + self.cell_h.saturating_sub(1)) as usize;
        let x0 = self.cols[col];
        [
            ((x0 + self.cell_w / 4) as usize, y),
            ((x0 + self.cell_w / 2) as usize, y),
            ((x0 + self.cell_w * 3 / 4) as usize, y),
        ]
    }

    /// True when at least 2 of 3 probes match the calibrated highlight color.
    /// Two-of-three tolerates one probe landing on overlapping item art.
    /// Off-frame probes count as non-matching.
    pub fn is_highlighted(&self, frame: &ScreenshotData, col: usize, row: usize) -> bool {
        self.probes(col, row)
            .iter()
            .filter(|(x, y)| {
                frame.try_get_pixel(*x, *y) == Some(self.highlight_color)
                    || (*y > 0 && frame.try_get_pixel(*x, *y - 1) == Some(self.highlight_color))
            })
            .count()
            >= 2
    }

    /// Build from the two calibrated corner cells by linear interpolation.
    pub fn from_corners(
        top_left: Rect,
        bottom_right: Rect,
        highlight_color: u32,
    ) -> anyhow::Result<Self> {
        if bottom_right.x <= top_left.x || bottom_right.y <= top_left.y {
            anyhow::bail!(
                "corner cells are not top-left/bottom-right ordered ({:?} vs {:?}) — \
                 the two calibration captures were taken in the wrong order; re-run calibrate-stash",
                top_left, bottom_right
            );
        }
        let cell_w = top_left.width;
        let cell_h = top_left.height;
        let pitch_x = (bottom_right.x - top_left.x) as f64 / (QUAD_COLS - 1) as f64;
        let pitch_y = (bottom_right.y - top_left.y) as f64 / (QUAD_ROWS - 1) as f64;
        if (pitch_x - cell_w as f64).abs() > 2.0 || (pitch_y - cell_h as f64).abs() > 2.0 {
            anyhow::bail!(
                "corner item spans more than one cell (cell {}x{} but pitch {:.1}x{:.1}) — \
                 use a 1x1 item for calibration",
                cell_w, cell_h, pitch_x, pitch_y
            );
        }
        let cols: [u32; QUAD_COLS] = std::array::from_fn(|i| {
            (top_left.x as f64 + pitch_x * i as f64).round() as u32
        });
        let rows: [u32; QUAD_ROWS] = std::array::from_fn(|i| {
            (top_left.y as f64 + pitch_y * i as f64).round() as u32
        });
        Ok(StashGrid {
            cols,
            rows,
            cell_w,
            cell_h,
            highlight_color,
        })
    }
}
