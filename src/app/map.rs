//! Map macro: click a cell of the calibrated 12x7 map grid.

use super::App;
use anyhow::bail;

impl App {
    /// Click cell (col, row) of the calibrated map grid.
    pub(crate) fn click_map_cell(&self, col: usize, row: usize) -> anyhow::Result<()> {
        self.focus_game_window()?;
        let settings = self.settings.read().clone();
        let grid = match &settings.map_grid {
            Some(g) => g.clone(),
            None => bail!("Map grid not calibrated — run: little_oil calibrate-map"),
        };
        let frame = self.screenshot()?;
        let (px, py) = grid.cell_center(col, row);
        let (sx, sy) = frame.frame_to_screen(px, py);
        self.click(sx, sy);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_helpers::{grid, grid_settings};
    use crate::platform::InputButton;
    use crate::stash_grid::MapGrid;
    use crate::test_support::{Ev, blank_frame, test_app};

    #[test]
    fn click_map_cell_clicks_the_calibrated_cell() {
        let (mut settings, region, _stash) = grid_settings();
        let map: MapGrid = grid(10, 0);
        settings.map_grid = Some(map.clone());
        let (app, fake) = test_app(settings);
        fake.push_frame(blank_frame(region, 0));

        app.click_map_cell(3, 4).expect("click the map cell");

        let (x, y) = map.cell_center(3, 4);
        let expected = vec![
            Ev::Sleep(100),
            Ev::Move(region.x as i32 + x as i32, region.y as i32 + y as i32),
            Ev::Sleep(30),
            Ev::Button(InputButton::Left, true),
            Ev::Sleep(10),
            Ev::Button(InputButton::Left, false),
        ];
        assert_eq!(fake.events(), expected);
    }
}
