//! Fixtures shared by the per-feature test modules in `src/app/`.

use crate::platform::{InputButton, InputKey};
use crate::screenshot::ScreenshotData;
use crate::stash_grid::{CellGrid, StashGrid};
use crate::test_support::{Ev, fill_rect};
use crate::{ScreenRegion, Settings};

/// Opaque colour of an empty inventory slot in synthetic frames.
pub(crate) const EMPTY: u32 = 0x101010FF;
/// Opaque colour of an occupied inventory slot in synthetic frames.
pub(crate) const ITEM: u32 = 0xFF0000FF;

/// The synthetic inventory occupies 12x5 slots of 100x100 px.
pub(crate) fn inv_region() -> ScreenRegion {
    ScreenRegion {
        x: 100,
        y: 200,
        width: 1200,
        height: 500,
    }
}

/// Screen centre of inventory cell (`col`, `row`): the middle probe point,
/// which is where the empty macro clicks.
pub(crate) fn cell_center(col: u32, row: u32) -> (i32, i32) {
    let r = inv_region();
    (
        (r.x + col * r.width / 12 + r.width / 24) as i32,
        (r.y + row * r.height / 5 + r.height / 10) as i32,
    )
}

/// Settings for the inventory macros: calibrated regions, every slot
/// empty, and no focus clicks so click assertions see only macro clicks.
pub(crate) fn inventory_settings() -> Settings {
    let mut s = crate::default_settings();
    s.focus_clicks = 0;
    s.game_window_region = Some(inv_region());
    s.inv_region = Some(inv_region());
    s.inv_samples = Some(vec![[EMPTY; 3]; 60]);
    s
}

/// A uniform `step`-pixel grid whose highlighted cells are painted with
/// `highlight` in the synthetic frames.
pub(crate) fn grid<const C: usize, const R: usize>(step: u32, highlight: u32) -> CellGrid<C, R> {
    CellGrid {
        cols: std::array::from_fn(|i| i as u32 * step),
        rows: std::array::from_fn(|i| i as u32 * step),
        cell_w: step,
        cell_h: step,
        highlight_color: highlight,
    }
}

/// Paint a cell's bottom border — the row `is_highlighted` probes.
pub(crate) fn highlight_cell(frame: &mut ScreenshotData, grid: &StashGrid, col: usize, row: usize) {
    let y = grid.rows[row] + grid.cell_h - 2;
    fill_rect(
        frame,
        grid.cols[col],
        y,
        grid.cell_w,
        2,
        grid.highlight_color,
    );
}

/// Settings for the stash/map macros: a 300x300 window at (100, 200) with
/// a 10-px grid, no focus clicks.
pub(crate) fn grid_settings() -> (Settings, ScreenRegion, StashGrid) {
    let region = ScreenRegion {
        x: 100,
        y: 200,
        width: 300,
        height: 300,
    };
    let grid: StashGrid = grid(10, 0xFF00FFFF);
    let mut s = crate::default_settings();
    s.focus_clicks = 0;
    s.game_window_region = Some(region);
    s.stash_grid = Some(grid.clone());
    (s, region, grid)
}

/// Screen positions of every press of `button`, taken from the `Move` that
/// immediately precedes it.
pub(crate) fn pressed_at(events: &[Ev], button: InputButton) -> Vec<(i32, i32)> {
    let mut last_move = None;
    let mut out = Vec::new();
    for ev in events {
        match ev {
            Ev::Move(x, y) => last_move = Some((*x, *y)),
            Ev::Button(b, true) if *b == button => {
                out.push(last_move.expect("every press follows a move"));
            }
            _ => {}
        }
    }
    out
}

pub(crate) fn count_key(events: &[Ev], key: InputKey, pressed: bool) -> usize {
    events
        .iter()
        .filter(|e| **e == Ev::Key(key, pressed))
        .count()
}

pub(crate) fn count_sleeps(events: &[Ev]) -> usize {
    events.iter().filter(|e| matches!(e, Ev::Sleep(_))).count()
}
