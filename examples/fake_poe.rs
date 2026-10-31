//! A fake "Path of Exile" window for the headless integration tests.
//!
//! It draws a synthetic 12×5 inventory panel in flat, exactly-known colors at a
//! fixed offset inside the window, so a test can
//!
//! * point `inv_region` at the panel,
//! * run `little_oil reset_inv` against an empty inventory and
//!   `little_oil empty` against a full one, and
//! * read back which cells were clicked.
//!
//! The inventory is a solid `EMPTY_SLOT` rectangle; the first `n` cells
//! (column-major, index `col * 5 + row`) are painted `ITEM` when `n` items are
//! shown. Both colors are far apart, so `occupied_inv_cells`' exact
//! probe-vs-sample comparison works on real composited pixels.
//!
//! Protocol (`stdout`, one line each, flushed immediately):
//!
//! * `READY` — first frame painted.
//! * `MOD ctrl=<0|1>` — the Ctrl state changed (lets a test wait for the
//!   harness to hold Ctrl before it runs the macro).
//! * `CLICK <button> <x> <y> ctrl=<0|1> cell=<col>,<row>` — one line per
//!   pointer press, `x`/`y` in window-content points, `cell=-` outside the
//!   panel. The item in the clicked cell is removed when Ctrl is held (the
//!   in-game "Ctrl+click moves the item" behavior); without Ctrl the item is
//!   only highlighted, matching a plain click that picks it up.
//!
//! The inventory is read from the file named by `FAKE_POE_ITEMS` (default
//! `fake_poe_items`): its first whitespace-separated token is
//!
//! * a number `n` — the first `n` cells, column-major (0 renders an empty
//!   inventory; that is what `little_oil reset_inv` samples),
//! * `diagonal` — cells (0,0), (3,1), (6,2), (9,3), (11,4), which exercises
//!   both axes and non-contiguous slots, or
//! * `quit` — close the window.
//!
//! The file is only re-applied when its contents change, so items removed by a
//! click stay removed until the test rewrites it.
//!
//! The window is undecorated and asks to be placed at the top-left corner, so
//! the inventory panel is at a fixed offset of the client area on every
//! platform; the tests still read the real window geometry from the compositor
//! or Win32 rather than assuming the request was honored.

use std::io::Write as _;
use std::time::Duration;

use eframe::egui;

/// Window content size requested at startup. Sway may resize it; the panel
/// stays at [`INV_OFFSET`] either way.
const WINDOW_SIZE: [f32; 2] = [1280.0, 720.0];

/// Top-left corner of the inventory panel inside the window.
const INV_OFFSET: (f32, f32) = (40.0, 40.0);
const INV_COLS: usize = 12;
const INV_ROWS: usize = 5;
const SLOT_W: f32 = 100.0;
const SLOT_H: f32 = 120.0;
/// Cell inset for the item square, so the probes at 25/50/75 % of a slot's
/// width and half its height always land on the item color.
const ITEM_INSET: f32 = 8.0;

const EMPTY_SLOT: egui::Color32 = egui::Color32::from_rgb(40, 40, 40);
const ITEM: egui::Color32 = egui::Color32::from_rgb(200, 120, 40);
const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(12, 12, 12);

fn main() -> eframe::Result {
    let items_path = std::env::var("FAKE_POE_ITEMS").unwrap_or_else(|_| "fake_poe_items".into());
    let options = eframe::NativeOptions {
        // Undecorated and pinned to the top-left corner so the Windows test can
        // use fixed screen coordinates (with no title bar the outer window and
        // the client area coincide). Wayland ignores the position; the sway
        // test reads the real geometry from the compositor instead.
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(WINDOW_SIZE)
            .with_position([0.0, 0.0])
            .with_decorations(false)
            .with_title("Path of Exile"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Path of Exile",
        options,
        Box::new(move |_cc| Ok(Box::new(FakePoe::new(items_path)))),
    )
}

/// The cells holding an item when `count` items are shown, column-major.
fn item_cells(count: usize) -> Vec<(usize, usize)> {
    (0..count.min(INV_COLS * INV_ROWS))
        .map(|i| (i / INV_ROWS, i % INV_ROWS))
        .collect()
}

/// The cells of the `diagonal` control-file pattern: one item on each row, in
/// different columns, so a test that reads back click coordinates catches an
/// x/y swap and non-contiguous slots.
fn diagonal_cells() -> Vec<(usize, usize)> {
    vec![(0, 0), (3, 1), (6, 2), (9, 3), (11, 4)]
}

/// Parse a control-file token into the inventory it describes.
fn inventory_for(token: &str) -> Option<Vec<(usize, usize)>> {
    if token == "diagonal" {
        return Some(diagonal_cells());
    }
    token.parse::<usize>().ok().map(item_cells)
}

/// The cell under a window-content point, or `None` outside the panel.
fn cell_at(pos: egui::Pos2) -> Option<(usize, usize)> {
    let x = pos.x - INV_OFFSET.0;
    let y = pos.y - INV_OFFSET.1;
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let col = (x / SLOT_W) as usize;
    let row = (y / SLOT_H) as usize;
    (col < INV_COLS && row < INV_ROWS).then_some((col, row))
}

struct FakePoe {
    items_path: String,
    items: Vec<(usize, usize)>,
    /// Verbatim control-file contents last applied. A click removes items
    /// permanently, so the file is only re-applied when it actually changes —
    /// otherwise the poll would undo the click on the next frame.
    last_control: String,
    ready: bool,
    ctrl: bool,
}

impl FakePoe {
    fn new(items_path: String) -> Self {
        Self {
            items_path,
            items: Vec::new(),
            last_control: String::new(),
            ready: false,
            ctrl: false,
        }
    }

    /// Re-read the control file: a count, or `quit`. Missing or unparseable
    /// contents mean "keep the current inventory".
    fn poll_control_file(&mut self, ctx: &egui::Context) {
        let Ok(text) = std::fs::read_to_string(&self.items_path) else {
            return;
        };
        if text == self.last_control {
            return;
        }
        self.last_control = text.clone();
        let Some(token) = text.split_whitespace().next() else {
            return;
        };
        if token == "quit" {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if let Some(items) = inventory_for(token) {
            self.items = items;
        }
    }

    /// Print one `CLICK` line per pointer press this frame, removing the item
    /// in a Ctrl-held click.
    ///
    /// Raw `Event::PointerButton` records are used instead of
    /// `pointer.button_pressed` so two clicks inside one frame (the empty macro
    /// clicks at ~20 ms intervals) are both reported.
    fn handle_clicks(&mut self, ctx: &egui::Context) {
        let mut presses = Vec::new();
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                {
                    presses.push((*pos, *button, modifiers.ctrl));
                }
            }
        });
        for (pos, button, ctrl) in presses {
            let cell = cell_at(pos);
            let label = match cell {
                Some((col, row)) => format!("{col},{row}"),
                None => "-".to_string(),
            };
            println!(
                "CLICK {} {} {} ctrl={} cell={label}",
                match button {
                    egui::PointerButton::Primary => "left",
                    egui::PointerButton::Secondary => "right",
                    egui::PointerButton::Middle => "middle",
                    _ => "other",
                },
                pos.x.round() as i32,
                pos.y.round() as i32,
                u8::from(ctrl)
            );
            let _ = std::io::stdout().flush();
            if ctrl && let Some(cell) = cell {
                self.items.retain(|c| *c != cell);
            }
        }
    }

    fn paint(&self, ui: &egui::Ui) {
        let painter = ui.painter();
        painter.rect_filled(ui.max_rect(), 0.0, BACKGROUND);
        let origin = egui::pos2(INV_OFFSET.0, INV_OFFSET.1);
        let panel = egui::Rect::from_min_size(
            origin,
            egui::vec2(SLOT_W * INV_COLS as f32, SLOT_H * INV_ROWS as f32),
        );
        painter.rect_filled(panel, 0.0, EMPTY_SLOT);
        for &(col, row) in &self.items {
            let cell = egui::Rect::from_min_size(
                origin + egui::vec2(col as f32 * SLOT_W, row as f32 * SLOT_H),
                egui::vec2(SLOT_W, SLOT_H),
            );
            painter.rect_filled(cell.shrink(ITEM_INSET), 0.0, ITEM);
        }
    }
}

impl eframe::App for FakePoe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_control_file(&ctx);
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        if ctrl != self.ctrl {
            self.ctrl = ctrl;
            println!("MOD ctrl={}", u8::from(ctrl));
            let _ = std::io::stdout().flush();
        }
        self.handle_clicks(&ctx);
        self.paint(ui);
        if !self.ready {
            self.ready = true;
            println!("READY");
            let _ = std::io::stdout().flush();
        }
        // Keep repainting so the control file is polled and clicks are
        // acknowledged between the macro's 30 ms capture intervals.
        ctx.request_repaint_after(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_fill_column_major() {
        assert_eq!(item_cells(0), vec![]);
        assert_eq!(item_cells(3), vec![(0, 0), (0, 1), (0, 2)]);
        assert_eq!(
            item_cells(6),
            vec![(0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (1, 0)]
        );
        assert_eq!(item_cells(1000).len(), 60, "capped at the grid size");
    }

    #[test]
    fn diagonal_pattern_uses_distinct_rows_and_columns() {
        let cells = diagonal_cells();
        assert_eq!(cells.len(), 5);
        for (index, &(_, row)) in cells.iter().enumerate() {
            assert_eq!(row, index, "one item per row, in order");
        }
        let columns: std::collections::BTreeSet<usize> =
            cells.iter().map(|&(col, _)| col).collect();
        assert_eq!(columns.len(), cells.len(), "every item in its own column");
        assert!(columns.contains(&0) && columns.contains(&(INV_COLS - 1)));
        assert!(columns.iter().filter(|&&col| col > 0).count() >= 4);
        assert_eq!(inventory_for("diagonal"), Some(diagonal_cells()));
        assert_eq!(inventory_for("2"), Some(vec![(0, 0), (0, 1)]));
        assert_eq!(inventory_for("bogus"), None);
    }

    #[test]
    fn points_map_to_cells() {
        assert_eq!(cell_at(egui::pos2(40.0, 40.0)), Some((0, 0)));
        assert_eq!(cell_at(egui::pos2(139.9, 159.9)), Some((0, 0)));
        assert_eq!(cell_at(egui::pos2(140.0, 160.0)), Some((1, 1)));
        assert_eq!(cell_at(egui::pos2(1239.0, 639.0)), Some((11, 4)));
        assert_eq!(cell_at(egui::pos2(39.9, 40.0)), None, "left of the panel");
        assert_eq!(cell_at(egui::pos2(40.0, 640.0)), None, "below the panel");
        assert_eq!(
            cell_at(egui::pos2(1240.0, 40.0)),
            None,
            "right of the panel"
        );
    }
}
