use serde::{Deserialize, Serialize};
use crate::screenshot::{Rect, ScreenshotData};

/// Quad tab is 24x24; map tab selection grid is 12x7.
pub const QUAD_COLS: usize = 24;
pub const QUAD_ROWS: usize = 24;
pub const MAP_COLS: usize = 12;
pub const MAP_ROWS: usize = 7;

/// Calibrated grid geometry (quad tab, map tab, …), in frame-pixel space
/// (relative to the game window region). Convert to screen with
/// ScreenshotData::to_screen.
///
/// serde is implemented manually below: serde has no blanket impl for
/// const-generic arrays (`[T; N]` is only serializable for concrete N ≤ 32),
/// so a derived Serialize/Deserialize would not compile.
#[derive(Debug, Clone)]
pub struct CellGrid<const C: usize, const R: usize> {
    /// Left edge of each column. Exactly C entries.
    pub cols: [u32; C],
    /// Top edge of each row. Exactly R entries.
    pub rows: [u32; R],
    pub cell_w: u32,
    pub cell_h: u32,
    /// Search-highlight border color, as packed by ScreenshotData::get_pixel.
    pub highlight_color: u32,
}

pub type StashGrid = CellGrid<QUAD_COLS, QUAD_ROWS>;
pub type MapGrid = CellGrid<MAP_COLS, MAP_ROWS>;

impl<const C: usize, const R: usize> Serialize for CellGrid<C, R> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("CellGrid", 5)?;
        st.serialize_field("cols", &self.cols[..])?;
        st.serialize_field("rows", &self.rows[..])?;
        st.serialize_field("cell_w", &self.cell_w)?;
        st.serialize_field("cell_h", &self.cell_h)?;
        st.serialize_field("highlight_color", &self.highlight_color)?;
        st.end()
    }
}

impl<'de, const C: usize, const R: usize> Deserialize<'de> for CellGrid<C, R> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Helper {
            cols: Vec<u32>,
            rows: Vec<u32>,
            cell_w: u32,
            cell_h: u32,
            highlight_color: u32,
        }
        let h = Helper::deserialize(d)?;
        let cols: [u32; C] = h.cols.try_into().map_err(|v: Vec<u32>| {
            serde::de::Error::custom(format!("cols: expected {C} entries, got {}", v.len()))
        })?;
        let rows: [u32; R] = h.rows.try_into().map_err(|v: Vec<u32>| {
            serde::de::Error::custom(format!("rows: expected {R} entries, got {}", v.len()))
        })?;
        Ok(CellGrid { cols, rows, cell_w: h.cell_w, cell_h: h.cell_h, highlight_color: h.highlight_color })
    }
}

impl<const C: usize, const R: usize> CellGrid<C, R> {
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
        let pitch_x = (bottom_right.x - top_left.x) as f64 / (C - 1) as f64;
        let pitch_y = (bottom_right.y - top_left.y) as f64 / (R - 1) as f64;
        if (pitch_x - cell_w as f64).abs() > 2.0 || (pitch_y - cell_h as f64).abs() > 2.0 {
            anyhow::bail!(
                "corner item spans more than one cell (cell {}x{} but pitch {:.1}x{:.1}) — \
                 use a 1x1 item for calibration",
                cell_w, cell_h, pitch_x, pitch_y
            );
        }
        let cols: [u32; C] = std::array::from_fn(|i| {
            (top_left.x as f64 + pitch_x * i as f64).round() as u32
        });
        let rows: [u32; R] = std::array::from_fn(|i| {
            (top_left.y as f64 + pitch_y * i as f64).round() as u32
        });
        Ok(CellGrid {
            cols,
            rows,
            cell_w,
            cell_h,
            highlight_color,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_grid_from_corners_passes_1x1() {
        let tl = Rect { x: 100, y: 100, width: 40, height: 40 };
        let br = Rect { x: 100 + 40 * 11, y: 100 + 40 * 6, width: 40, height: 40 };
        let grid = MapGrid::from_corners(tl, br, 0xE7B477FF).unwrap();
        assert_eq!(grid.cols.len(), MAP_COLS);
        assert_eq!(grid.rows.len(), MAP_ROWS);
        assert_eq!(grid.cell_center(11, 6), (100 + 40 * 11 + 20, 100 + 40 * 6 + 20));
    }

    #[test]
    fn test_map_grid_from_corners_bails_on_multi_cell() {
        let tl = Rect { x: 100, y: 100, width: 80, height: 40 }; // 2 wide
        let br = Rect { x: 100 + 40 * 11, y: 100 + 40 * 6, width: 40, height: 40 };
        assert!(MapGrid::from_corners(tl, br, 0).is_err());
    }
}
