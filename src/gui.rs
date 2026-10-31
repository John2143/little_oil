//! `little_oil gui` — tray icon (Windows) + settings/calibration panel.
//!
//! The panel is an eframe/egui window. Long-running macro actions run on
//! background threads; results stream into a shared log. Every surface — this
//! panel, the Windows tray menu and global hotkeys, and the CLI — is driven by
//! the registry in [`crate::actions`]. On Linux the WM keybinds replace the
//! tray/hotkeys, so none are registered there.
//!
//! Tabs:
//! * **Actions** — one button per registry action, plus the log.
//! * **Setup** — guided first-run wizard: detect window, drag regions, sample
//!   inventory colors, calibrate pointer.
//! * **Calibrate** — capture the game window, drag a rectangle, save it as a
//!   region (game/inventory/stash/map), re-sample inventory colors.
//! * **Inventory** — capture + live occupied-slot overlay.
//! * **Health** — checkhealth-style diagnostics (also `little_oil doctor`).
//! * **Settings** — edit region numbers, pointer scale, focus clicks.

use crate::actions::{ACTIONS, Action};
use crate::app::App;
use crate::screenshot::ScreenshotData;
use crate::{ScreenRegion, config_path};
use anyhow::Result;
use eframe::egui;
use parking_lot::Mutex;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Which region the next completed drag-select should be saved to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RegionTarget {
    Game,
    Inventory,
    Stash,
    Map,
}

impl RegionTarget {
    fn label(self) -> &'static str {
        match self {
            RegionTarget::Game => "Game window",
            RegionTarget::Inventory => "Inventory grid",
            RegionTarget::Stash => "Stash grid",
            RegionTarget::Map => "Map grid",
        }
    }
}

/// Shared state between the UI thread and background action threads.
struct GuiState {
    log: VecDeque<String>,
}

impl GuiState {
    fn push(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::info!(gui = true, "{msg}");
        self.log.push_back(msg);
        while self.log.len() > 300 {
            self.log.pop_front();
        }
    }
}

/// A captured screenshot ready to display.
struct Preview {
    data: ScreenshotData,
    texture: egui::TextureHandle,
    /// Display size in points (fits the panel).
    shown: egui::Vec2,
    /// Where the image is actually drawn this frame (set by the tab that
    /// renders it) — used to map display-space drags to image pixels.
    rect: egui::Rect,
}

impl Preview {
    /// The pure conversion data for drags in this preview.
    fn geometry(&self) -> PreviewGeometry {
        PreviewGeometry::new(
            (self.data.width as u32, self.data.height as u32),
            self.data.origin,
            self.shown,
        )
    }
}

/// Geometry of a captured desktop as drawn in a GUI preview: image size in
/// pixels, the screen coordinate of its top-left pixel, and its display size
/// in points. All conversions are pure so they can be unit-tested without a
/// window; drag coordinates are relative to the image's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreviewGeometry {
    /// Captured image size in pixels.
    pub image: (u32, u32),
    /// Screen coordinate of image pixel (0, 0).
    pub origin: (i32, i32),
    /// Display size of the image in points.
    pub shown: egui::Vec2,
}

impl PreviewGeometry {
    pub(crate) fn new(image: (u32, u32), origin: (i32, i32), shown: egui::Vec2) -> Self {
        Self {
            image,
            origin,
            shown,
        }
    }

    /// Image pixel under a display-space point. `None` outside the image or
    /// for an empty image.
    pub(crate) fn image_point(&self, at: (f32, f32)) -> Option<(u32, u32)> {
        let (w, h) = (self.image.0 as f32, self.image.1 as f32);
        if w <= 0.0 || h <= 0.0 || self.shown.x <= 0.0 || self.shown.y <= 0.0 {
            return None;
        }
        let px = at.0 * w / self.shown.x;
        let py = at.1 * h / self.shown.y;
        if !(0.0..=w).contains(&px) || !(0.0..=h).contains(&py) {
            return None;
        }
        Some((px as u32, py as u32))
    }

    /// Screen coordinate of a display-space point.
    pub(crate) fn screen_point(&self, at: (f32, f32)) -> Option<(i32, i32)> {
        let (ix, iy) = self.image_point(at)?;
        Some((self.origin.0 + ix as i32, self.origin.1 + iy as i32))
    }

    /// Screen-space region for a drag between two display-space points.
    ///
    /// The drag can go in any direction; the result is at least 1×1. Negative
    /// screen coordinates (a monitor left of / above the primary one) are
    /// clamped to 0 because [`ScreenRegion`] coordinates are unsigned.
    pub(crate) fn region_from_drag(&self, a: (f32, f32), b: (f32, f32)) -> Option<ScreenRegion> {
        let lo = (a.0.min(b.0), a.1.min(b.1));
        let hi = (a.0.max(b.0), a.1.max(b.1));
        let (x0, y0) = self.image_point(lo)?;
        let (x1, y1) = self.image_point(hi)?;
        Some(ScreenRegion {
            x: (self.origin.0 + x0 as i32).max(0) as u32,
            y: (self.origin.1 + y0 as i32).max(0) as u32,
            width: x1.saturating_sub(x0).max(1),
            height: y1.saturating_sub(y0).max(1),
        })
    }
}

/// Wizard progress. `step` is the current wizard step index (0..=5).
struct SetupState {
    step: usize,
    /// Result of the last "Detect game window" press.
    detected_window: Option<ScreenRegion>,
    /// True once the user pressed "Detect game window" — "Not found" only
    /// shows after an actual attempt.
    detect_tried: bool,
    /// Wizard step a desktop capture was last auto-attempted for: one attempt
    /// per step entry, so a failing capture doesn't spam the log every frame.
    last_captured_step: Option<usize>,
}

/// Starts macro actions on worker threads. Cheap to clone into the tray and
/// hotkey event handlers, which run outside the egui frame callbacks.
#[derive(Clone)]
struct ActionRunner {
    app: Arc<App>,
    state: Arc<Mutex<GuiState>>,
    busy: Arc<AtomicBool>,
    ctx: egui::Context,
}

impl ActionRunner {
    fn push(&self, msg: impl Into<String>) {
        self.state.lock().push(msg);
        self.ctx.request_repaint();
    }

    /// Run `f` on a background thread unless another action is running.
    fn run(&self, name: &'static str, f: Action) {
        if self.busy.swap(true, Ordering::AcqRel) {
            self.push(format!(
                "{name}: already busy — wait for the current action"
            ));
            return;
        }
        self.push(format!("▶ {name}…"));
        let this = self.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = f(&this.app);
            let elapsed = started.elapsed();
            match result {
                Ok(()) => this.push(format!("✓ {name} done in {:.1}s", elapsed.as_secs_f32())),
                Err(e) => this.push(format!("✗ {name} failed: {e:#}")),
            }
            this.busy.store(false, Ordering::Release);
            this.ctx.request_repaint();
        });
    }
}

struct LittleOilGui {
    app: Arc<App>,
    runner: ActionRunner,

    tab: &'static str,
    preview: Option<Preview>,
    /// Drag rectangle in *display* space (shown image coords), when active.
    drag: Option<(egui::Pos2, egui::Pos2)>,
    /// Screen point of the last right-click on the preview (for the
    /// "select whole window/monitor" context menu).
    right_click_screen: Option<(i32, i32)>,
    region_target: RegionTarget,
    show_inv_overlay: bool,
    status: String,
    setup: SetupState,
    health_checks: Option<Vec<crate::health::Check>>,
    last_tab: &'static str,

    // Windows-only: system tray + global hotkeys. Kept here (the event-loop
    // thread) because both are !Send/!Sync on Windows.
    #[cfg(target_os = "windows")]
    tray: Option<tray_icon::TrayIcon>,
    #[cfg(target_os = "windows")]
    hotkeys: Option<Hotkeys>,
    /// The binding list the current `hotkeys` manager was built from, so a
    /// changed setting re-registers exactly once.
    #[cfg(target_os = "windows")]
    hotkey_spec: String,
}

/// Draw one button per registry action and report the ones pressed this frame.
///
/// Free function (no `LittleOilGui`) so the Actions tab can be driven by
/// `egui_kittest` without an eframe window.
fn action_buttons(ui: &mut egui::Ui, busy: bool, mut on_click: impl FnMut(&'static str, Action)) {
    ui.horizontal_wrapped(|ui| {
        for def in ACTIONS {
            if ui
                .add_enabled(!busy, egui::Button::new(def.label))
                .on_hover_text(def.help)
                .clicked()
            {
                on_click(def.id, def.run);
            }
        }
    });
}

impl LittleOilGui {
    fn new(cc: &eframe::CreationContext<'_>, app: Arc<App>) -> Self {
        let state = Arc::new(Mutex::new(GuiState {
            log: VecDeque::new(),
        }));
        let first_run = !app.settings.read().setup_complete;
        let tab = if first_run { "Setup" } else { "Actions" };
        let mut this = LittleOilGui {
            runner: ActionRunner {
                app: Arc::clone(&app),
                state,
                busy: Arc::new(AtomicBool::new(false)),
                ctx: cc.egui_ctx.clone(),
            },
            app,
            tab,
            preview: None,
            drag: None,
            right_click_screen: None,
            region_target: RegionTarget::Game,
            show_inv_overlay: false,
            status: String::new(),
            setup: SetupState {
                step: 0,
                detected_window: None,
                detect_tried: false,
                last_captured_step: None,
            },
            health_checks: None,
            last_tab: tab,
            #[cfg(target_os = "windows")]
            tray: None,
            #[cfg(target_os = "windows")]
            hotkeys: None,
            #[cfg(target_os = "windows")]
            hotkey_spec: String::new(),
        };
        this.status = this.summary_line();
        this
    }

    fn summary_line(&self) -> String {
        let s = self.app.settings.read();
        let desktop = s
            .desktop
            .map(|d| format!("{d:?}"))
            .unwrap_or_else(|| "auto".into());
        let inv = s
            .inv_region
            .map(|r| format!("{}x{}", r.width, r.height))
            .unwrap_or_else(|| "not set".into());
        format!(
            "desktop: {desktop} · inventory: {inv} · samples: {}",
            s.inv_samples.as_ref().map(|v| v.len()).unwrap_or(0)
        )
    }

    fn push(&mut self, msg: impl Into<String>) {
        self.runner.push(msg);
    }

    /// Run a macro action on a background thread, guarding against overlap.
    fn run_action(&mut self, name: &'static str, f: Action) {
        self.runner.run(name, f);
    }

    fn capture_preview(&mut self, ctx: &egui::Context) {
        // Calibration preview: capture the whole desktop, not the current
        // game-window region, so any region can be dragged and selected.
        match self.app.backend().capture_desktop() {
            Ok(data) => {
                let (w, h) = (data.width, data.height);
                let (ox, oy) = data.origin;
                let size = egui::Vec2::new(w as f32, h as f32);
                let color = egui::ColorImage::from_rgba_unmultiplied([w, h], &data.pixels);
                let texture = ctx.load_texture("capture", color, egui::TextureOptions::NEAREST);
                let shown = fit_into(size, PREVIEW_AVAIL);
                self.preview = Some(Preview {
                    data,
                    texture,
                    shown,
                    rect: egui::Rect::ZERO, // set by the tab that draws it
                });
                self.push(format!("captured desktop {w}x{h} (origin {ox}, {oy})"));
            }
            Err(e) => self.push(format!("capture failed: {e:#}")),
        }
    }

    /// A display-space point as an offset from the image's top-left corner.
    fn image_relative(&self, p: egui::Pos2) -> Option<(f32, f32)> {
        let rect = self.preview.as_ref()?.rect;
        Some((p.x - rect.min.x, p.y - rect.min.y))
    }

    /// Convert a drag rect in display space to a screen-space region.
    fn drag_to_region(&self, a: egui::Pos2, b: egui::Pos2) -> Option<ScreenRegion> {
        let a = self.image_relative(a)?;
        let b = self.image_relative(b)?;
        self.preview.as_ref()?.geometry().region_from_drag(a, b)
    }

    /// Display-space point → screen coordinates (for the right-click menu).
    fn screen_point_of(&self, p: egui::Pos2) -> Option<(i32, i32)> {
        let (x, y) = self.image_relative(p)?;
        self.preview.as_ref()?.geometry().screen_point((x, y))
    }

    /// Write `region` into the settings field for `target` and persist.
    fn apply_selected_region(&mut self, target: RegionTarget, region: ScreenRegion, what: &str) {
        {
            let mut s = self.app.settings.write();
            match target {
                RegionTarget::Game => s.game_window_region = Some(region),
                RegionTarget::Inventory => s.inv_region = Some(region),
                RegionTarget::Stash => s.stash_region = Some(region),
                RegionTarget::Map => s.map_region = Some(region),
            }
        }
        match self.app.save_settings() {
            Ok(()) => self.push(format!(
                "{what} → {} region: {}x{} at ({}, {})",
                target.label(),
                region.width,
                region.height,
                region.x,
                region.y
            )),
            Err(e) => self.push(format!("saved in memory but config write failed: {e:#}")),
        }
    }

    fn save_dragged_region(&mut self) {
        let Some((a, b)) = self.drag else { return };
        self.drag = None;
        let Some(region) = self.drag_to_region(a, b) else {
            self.push("drag was outside the captured image — try again");
            return;
        };
        let target = self.region_target;
        self.apply_selected_region(target, region, "drag");
    }

    /// Right-click menu: select the whole window under the cursor.
    fn select_whole_window(&mut self, sx: i32, sy: i32) {
        match self.app.backend().window_at(sx, sy) {
            Some(region) => self.apply_selected_region(self.region_target, region, "window"),
            None => self.push("no window found under the cursor (unsupported compositor?)"),
        }
    }

    /// Right-click menu: select the whole monitor under the cursor.
    fn select_whole_monitor(&mut self, sx: i32, sy: i32) {
        match self.app.backend().monitor_at(sx, sy) {
            Some(region) => self.apply_selected_region(self.region_target, region, "monitor"),
            None => self.push("no monitor found under the cursor"),
        }
    }

    fn ui_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.runner.busy.load(Ordering::Acquire);
        ui.add_space(8.0);
        action_buttons(ui, busy, |id, run| self.run_action(id, run));
        ui.add_space(8.0);
        if busy {
            ui.colored_label(egui::Color32::from_rgb(220, 160, 60), "⏳ action running…");
        }
        ui.separator();
        ui.heading("Log");
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let log = self.runner.state.lock();
                for line in log.log.iter().rev().take(50).rev() {
                    ui.monospace(line);
                }
            });
    }

    fn ui_calibrate(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Calibrate");
        ui.horizontal(|ui| {
            if ui
                .button("Capture game window")
                .on_hover_text("Capture the whole desktop so any region can be dragged out")
                .clicked()
            {
                self.capture_preview(ctx);
            }
            ui.label("then drag a rectangle below and save it as:");
            let combo = egui::ComboBox::from_id_salt("region_target")
                .selected_text(self.region_target.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.region_target,
                        RegionTarget::Game,
                        RegionTarget::Game.label(),
                    );
                    ui.selectable_value(
                        &mut self.region_target,
                        RegionTarget::Inventory,
                        RegionTarget::Inventory.label(),
                    );
                    ui.selectable_value(
                        &mut self.region_target,
                        RegionTarget::Stash,
                        RegionTarget::Stash.label(),
                    );
                    ui.selectable_value(
                        &mut self.region_target,
                        RegionTarget::Map,
                        RegionTarget::Map.label(),
                    );
                });
            combo
                .response
                .on_hover_text("Which region the selection saves to");
            if ui
                .button("Save selection")
                .on_hover_text("Save the dragged rectangle to the selected region")
                .clicked()
            {
                self.save_dragged_region();
            }
        });
        ui.add_space(4.0);
        ui.label(
            "Drag to select a region. Right-click the preview to select a whole window or monitor.",
        );
        if !self.preview_and_drag(ui) {
            ui.weak("No capture yet — click \"Capture game window\".");
        }
        ui.add_space(8.0);
        let s = self.app.settings.read();
        if let Some(r) = s.game_window_region {
            ui.label(format!(
                "game: {}x{} @ ({}, {})",
                r.width, r.height, r.x, r.y
            ));
        }
        if let Some(r) = s.inv_region {
            ui.label(format!(
                "inventory: {}x{} @ ({}, {})",
                r.width, r.height, r.x, r.y
            ));
        }
        if s.inv_samples.as_ref().map(|v| v.len()) != Some(60) {
            ui.colored_label(
                egui::Color32::from_rgb(220, 160, 60),
                "Inventory colors not sampled — run \"Recalibrate inventory colors\" after setting the inventory region.",
            );
        }
    }

    /// Render the captured desktop and handle drag-select + right-click
    /// selection (whole window/monitor). Returns false when there is no
    /// preview — the caller shows its own hint.
    fn preview_and_drag(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(_) = &self.preview else {
            return false;
        };
        // Short borrows only — the interaction handlers below take &mut self.
        let shown = self.preview.as_ref().map(|p| p.shown).unwrap_or_default();
        let (rect, response) = ui.allocate_exact_size(shown, egui::Sense::click_and_drag());
        if let Some(prev) = &mut self.preview {
            prev.rect = rect;
        }
        if let Some(prev) = &self.preview {
            ui.painter().image(
                prev.texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        if response.drag_started()
            && let Some(anchor) = ui.input(|i| i.pointer.press_origin())
        {
            // Anchor on the press position, not the point where egui decides
            // the press became a drag (that is already past the threshold, so
            // a fast drag would start from the wrong corner).
            let cur = response.interact_pointer_pos().unwrap_or(anchor);
            self.drag = Some((anchor, cur));
        }
        if response.dragged()
            && let (Some(drag), Some(cur)) = (self.drag.as_mut(), response.interact_pointer_pos())
        {
            drag.1 = cur;
        }
        if response.drag_stopped() {
            if let Some(cur) = response.interact_pointer_pos() {
                let anchor = self.drag.unwrap_or((cur, cur)).0;
                self.drag = Some((anchor, cur));
            }
            self.save_dragged_region();
        }
        if response.secondary_clicked() {
            self.right_click_screen = response
                .interact_pointer_pos()
                .and_then(|p| self.screen_point_of(p));
        }
        // Right-click context menu: select the whole window or monitor.
        response.context_menu(|ui| {
            if ui.button("Select whole window under cursor").clicked() {
                if let Some((sx, sy)) = self.right_click_screen {
                    self.select_whole_window(sx, sy);
                }
                ui.close();
            }
            if ui.button("Select whole monitor under cursor").clicked() {
                if let Some((sx, sy)) = self.right_click_screen {
                    self.select_whole_monitor(sx, sy);
                }
                ui.close();
            }
        });
        // Draw the drag overlay.
        if let Some((a, b)) = self.drag {
            let r = egui::Rect::from_two_pos(a, b);
            ui.painter().rect_stroke(
                r,
                0.0,
                egui::Stroke::new(2.0_f32, egui::Color32::LIGHT_BLUE),
                egui::StrokeKind::Outside,
            );
            ui.painter().text(
                r.max + egui::vec2(4.0, 0.0),
                egui::Align2::LEFT_TOP,
                self.region_target.label(),
                egui::FontId::monospace(12.0),
                egui::Color32::LIGHT_BLUE,
            );
        }
        true
    }

    fn ui_inventory(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Inventory");
        ui.horizontal(|ui| {
            if ui
                .button("Capture")
                .on_hover_text("Capture the whole desktop to preview the inventory overlay")
                .clicked()
            {
                self.capture_preview(ctx);
            }
            ui.checkbox(&mut self.show_inv_overlay, "overlay occupied slots")
                .on_hover_text("Highlight slots detected as occupied");
        });
        if let Some(prev) = &self.preview {
            let (rect, _) = ui.allocate_exact_size(prev.shown, egui::Sense::hover());
            ui.painter().image(
                prev.texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            if self.show_inv_overlay {
                self.draw_inv_overlay(ui, rect, prev);
            }
        } else {
            ui.weak("No capture yet — click \"Capture\".");
        }
    }

    fn draw_inv_overlay(&self, ui: &egui::Ui, rect: egui::Rect, preview: &Preview) {
        let settings = self.app.settings.read();
        let Some(inv) = settings.inv_region else {
            return;
        };
        let Some(samples) = settings.inv_samples.clone() else {
            return;
        };
        if samples.len() != 60 {
            return;
        }
        let frame = &preview.data;
        // Inv region relative to the frame origin.
        let rx = inv.x as i64 - frame.origin.0 as i64;
        let ry = inv.y as i64 - frame.origin.1 as i64;
        if rx < 0 || ry < 0 {
            return;
        }
        let (rx, ry) = (rx as u32, ry as u32);
        let cell_w = inv.width / 12;
        let cell_h = inv.height / 5;
        let scale = egui::Vec2::new(
            preview.shown.x / preview.data.width as f32,
            preview.shown.y / preview.data.height as f32,
        );
        let to_disp = |fx: u32, fy: u32| {
            egui::pos2(
                rect.min.x + fx as f32 * scale.x,
                rect.min.y + fy as f32 * scale.y,
            )
        };
        for row in 0..5u32 {
            for col in 0..12u32 {
                let cell_x = rx + col * cell_w;
                let cell_y = ry + row * cell_h;
                // occupied = fewer than 2 probe pixels match the sample
                let matches = App::inv_probes(frame, inv, col, row)
                    .map(|probes| {
                        probes
                            .iter()
                            .zip(samples[(col * 5 + row) as usize].iter())
                            .filter(|(p, s)| {
                                frame
                                    .try_get_pixel(p.0, p.1)
                                    .map(|px| px == **s)
                                    .unwrap_or(false)
                            })
                            .count()
                    })
                    .unwrap_or(0);
                let occupied = matches < 2;
                let p0 = to_disp(cell_x, cell_y);
                let p1 = to_disp(cell_x + cell_w, cell_y + cell_h);
                let cell = egui::Rect::from_two_pos(p0, p1);
                if occupied {
                    ui.painter().rect_filled(
                        cell,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(220, 60, 60, 110),
                    );
                }
                ui.painter().rect_stroke(
                    cell,
                    0.0,
                    egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 60),
                    ),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    /// Whether the wizard's pointer-calibration step should be shown at all.
    /// Absolute-positioning platforms (niri on Linux, Windows) never need it.
    #[cfg(target_os = "linux")]
    fn wizard_pointer_step_needed(&self) -> bool {
        !self.app.desktop().uses_absolute_pointer()
    }

    #[cfg(not(target_os = "linux"))]
    fn wizard_pointer_step_needed(&self) -> bool {
        false
    }

    /// Capture a fresh desktop preview once per wizard step entry, so a
    /// failing capture doesn't retry (and spam the log) every frame.
    fn auto_capture_once(&mut self, ctx: &egui::Context) {
        if self.preview.is_none() && self.setup.last_captured_step != Some(self.setup.step) {
            self.capture_preview(ctx);
            self.setup.last_captured_step = Some(self.setup.step);
        }
    }

    /// Guided first-run setup. Auto-selected when `setup_complete` is false;
    /// always re-runnable from the tab bar. Reuses the Calibrate tab's drag
    /// machinery (preview_and_drag) so the wizard and manual calibration can
    /// never drift apart.
    fn ui_setup(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Setup");
        if self.app.settings.read().setup_complete {
            ui.colored_label(
                egui::Color32::from_rgb(120, 200, 120),
                "✓ Setup complete — you can redo any step below.",
            );
        }
        ui.label(format!("Step {} of 5", self.setup.step + 1));
        ui.add_space(6.0);
        match self.setup.step {
            0 => {
                ui.label("Launch Path of Exile (any window mode) before continuing.");
                ui.add_space(4.0);
                if ui
                    .button("Detect game window")
                    .on_hover_text("Finds the Path of Exile window on this desktop")
                    .clicked()
                {
                    self.setup.detected_window = self.app.backend().game_window();
                    self.setup.detect_tried = true;
                }
                ui.add_space(4.0);
                match self.setup.detected_window {
                    Some(r) => {
                        ui.label(format!(
                            "Found: {}x{} @ ({}, {})",
                            r.width, r.height, r.x, r.y
                        ));
                        if ui.button("Use this window").clicked() {
                            if let Some(r) = self.setup.detected_window.take() {
                                self.apply_selected_region(RegionTarget::Game, r, "setup");
                            }
                            self.setup.step = 1;
                        }
                    }
                    None if self.setup.detect_tried => {
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 160, 60),
                            "Couldn't detect the game window on this desktop — drag a box around it.",
                        );
                    }
                    None => {}
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("Continue without detecting")
                        .on_hover_text("You'll drag the game window region on the next step")
                        .clicked()
                    {
                        self.setup.detected_window = None;
                        self.setup.detect_tried = false;
                        self.setup.step = 1;
                    }
                    if ui
                        .button("Skip setup")
                        .on_hover_text("Close the wizard — re-open it from the Setup tab anytime")
                        .clicked()
                    {
                        self.setup.step = 0;
                        self.setup.detected_window = None;
                        self.setup.detect_tried = false;
                        self.tab = "Actions";
                    }
                });
            }
            1 => {
                self.region_target = RegionTarget::Game;
                self.auto_capture_once(ctx);
                ui.label("Drag a rectangle around the whole game window, then press Continue.");
                ui.horizontal(|ui| {
                    if ui.button("Capture desktop").clicked() {
                        self.capture_preview(ctx);
                    }
                });
                ui.add_space(4.0);
                if !self.preview_and_drag(ui) {
                    ui.weak("No capture yet — press \"Capture desktop\".");
                }
                let game_set = self.app.settings.read().game_window_region.is_some();
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        self.setup.step = 0;
                    }
                    if ui
                        .add_enabled(game_set, egui::Button::new("Continue"))
                        .on_hover_text("Needs a game window region — drag one above first")
                        .clicked()
                    {
                        self.setup.step = 2;
                    }
                });
            }
            2 => {
                self.region_target = RegionTarget::Inventory;
                self.auto_capture_once(ctx);
                ui.label("Open your inventory in the game and leave it EMPTY, then drag a rectangle around the 12x5 grid.");
                ui.horizontal(|ui| {
                    if ui.button("Capture desktop").clicked() {
                        self.capture_preview(ctx);
                    }
                });
                ui.add_space(4.0);
                if !self.preview_and_drag(ui) {
                    ui.weak("No capture yet — press \"Capture desktop\".");
                }
                let inv_set = self.app.settings.read().inv_region.is_some();
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        self.setup.step = 1;
                    }
                    if ui
                        .add_enabled(inv_set, egui::Button::new("Continue"))
                        .on_hover_text("Needs an inventory region — drag one above first")
                        .clicked()
                    {
                        self.setup.step = 3;
                    }
                });
            }
            3 => {
                ui.label("Keep the inventory open and empty, then press Recalibrate. Do not move the mouse while it runs.");
                ui.add_space(4.0);
                if ui
                    .button("Recalibrate inventory colors")
                    .on_hover_text(
                        "Samples the 60 empty-slot probe colors from the inventory region",
                    )
                    .clicked()
                {
                    let def =
                        crate::actions::find("reset_inv").expect("reset_inv is a registry action");
                    self.run_action(def.id, def.run);
                }
                ui.add_space(4.0);
                let n = self
                    .app
                    .settings
                    .read()
                    .inv_samples
                    .as_ref()
                    .map(|v| v.len());
                match n {
                    Some(60) => {
                        ui.colored_label(egui::Color32::from_rgb(120, 200, 120), "60/60 — done");
                    }
                    Some(k) => {
                        ui.label(format!("samples: {k}/60"));
                    }
                    None => {
                        ui.weak("samples: 0/60 — not sampled yet");
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        self.setup.step = 2;
                    }
                    if ui
                        .add_enabled(n == Some(60), egui::Button::new("Continue"))
                        .on_hover_text(
                            "Needs 60 color samples — Recalibrate with an empty inventory first",
                        )
                        .clicked()
                    {
                        self.setup.step = if self.wizard_pointer_step_needed() {
                            4
                        } else {
                            5
                        };
                    }
                });
            }
            4 => {
                ui.label("Calibrates how far the mouse moves per click. It moves the mouse for ~3 seconds — close animated windows and don't touch the mouse.");
                ui.add_space(4.0);
                #[cfg(target_os = "linux")]
                if ui
                    .button("Calibrate pointer")
                    .on_hover_text("Measures pointer_scale by diffing cursor positions before/after a raw move")
                    .clicked()
                {
                    self.run_action("calibrate-pointer", |app| app.calibrate_pointer());
                }
                ui.add_space(4.0);
                if let Some(s) = self.app.settings.read().pointer_scale {
                    ui.label(format!("pointer_scale = {s:.4}"));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        self.setup.step = 3;
                    }
                    if ui
                        .button("Skip")
                        .on_hover_text("Skip calibration — re-run Setup or calibrate-pointer later")
                        .clicked()
                    {
                        self.setup.step = 5;
                    }
                    if ui.button("Continue").clicked() {
                        self.setup.step = 5;
                    }
                });
            }
            _ => {
                // Ask before taking the read guard: it re-reads settings.
                let pointer_needed = self.wizard_pointer_step_needed();
                let s = self.app.settings.read();
                let game = s
                    .game_window_region
                    .map(|r| format!("{}x{} @ ({}, {})", r.width, r.height, r.x, r.y))
                    .unwrap_or_else(|| "not set".into());
                let inv = s
                    .inv_region
                    .map(|r| format!("{}x{} @ ({}, {})", r.width, r.height, r.x, r.y))
                    .unwrap_or_else(|| "not set".into());
                let samples = s.inv_samples.as_ref().map(|v| v.len()).unwrap_or(0);
                let focus = s.focus_clicks;
                let scale = if pointer_needed {
                    s.pointer_scale
                        .map(|v| format!("{v:.4}"))
                        .unwrap_or_else(|| "not calibrated — redo step 4".into())
                } else {
                    "absolute positioning — not needed".into()
                };
                drop(s);
                ui.add_space(4.0);
                ui.label(format!("Game window: {game}"));
                ui.label(format!("Inventory grid: {inv}"));
                ui.label(format!("Color samples: {samples}/60"));
                ui.label(format!("Focus clicks: {focus}"));
                ui.label(format!("Pointer scale: {scale}"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        self.setup.step = if self.wizard_pointer_step_needed() {
                            4
                        } else {
                            3
                        };
                    }
                    if ui
                        .button("Finish")
                        .on_hover_text("Mark setup complete and open the Actions tab")
                        .clicked()
                    {
                        self.app.settings.write().setup_complete = true;
                        if let Err(e) = self.app.save_settings() {
                            self.push(format!("config write failed: {e:#}"));
                        } else {
                            self.push("Setup complete — you can use the Actions tab now");
                        }
                        self.setup.step = 0;
                        self.setup.detected_window = None;
                        self.setup.detect_tried = false;
                        self.tab = "Actions";
                    }
                });
            }
        }
    }

    /// checkhealth-style diagnostics — the same checks `little_oil doctor`
    /// runs, rendered as colored rows. Re-runs on every visit and on demand.
    fn ui_health(&mut self, ui: &mut egui::Ui) {
        ui.heading("Health");
        ui.label("Diagnostics, checkhealth-style. Fix any red items, then re-run.");
        if self.health_checks.is_none()
            || ui
                .button("Re-run checks")
                .on_hover_text("Re-run every health check now")
                .clicked()
        {
            self.health_checks = Some(crate::health::run(&self.app));
        }
        ui.add_space(6.0);
        let Some(checks) = &self.health_checks else {
            return;
        };
        for c in checks {
            let (color, tag) = match c.status {
                crate::health::CheckStatus::Good => (egui::Color32::from_rgb(120, 200, 120), "✓"),
                crate::health::CheckStatus::Warn => (egui::Color32::from_rgb(220, 160, 60), "△"),
                crate::health::CheckStatus::Error => (egui::Color32::from_rgb(220, 60, 60), "✗"),
            };
            ui.horizontal(|ui| {
                ui.colored_label(color, tag);
                ui.monospace(format!("{:<18}", c.name));
                ui.label(&c.message);
            });
        }
        if checks
            .iter()
            .any(|c| c.status == crate::health::CheckStatus::Error)
        {
            ui.add_space(6.0);
            ui.colored_label(
                egui::Color32::from_rgb(220, 60, 60),
                "Some checks failed — open the Setup tab to fix them.",
            );
        }
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.label("Regions are in screen pixels. Calibrate them with the drag-select on the Calibrate tab instead of typing numbers.");
        ui.add_space(6.0);
        let (mut focus_clicks, mut scale) = {
            let s = self.app.settings.read();
            (s.focus_clicks, s.pointer_scale.unwrap_or(1.25))
        };
        let mut dirty = false;
        egui::Grid::new("settings_grid")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Focus clicks:");
                if ui
                    .add(egui::DragValue::new(&mut focus_clicks).range(1..=5))
                    .on_hover_text("Clicks needed to focus the game window before a macro starts")
                    .changed()
                {
                    dirty = true;
                }
                ui.end_row();
                ui.label("Pointer scale:");
                if ui
                    .add(
                        egui::DragValue::new(&mut scale)
                            .speed(0.01)
                            .range(0.5..=3.0),
                    )
                    .on_hover_text(
                        "Multiplier applied to pointer movement (game-sensitivity offset)",
                    )
                    .changed()
                {
                    dirty = true;
                }
                ui.end_row();
            });
        if dirty {
            {
                let mut s = self.app.settings.write();
                s.focus_clicks = focus_clicks;
                s.pointer_scale = Some(scale);
            }
            match self.app.save_settings() {
                Ok(()) => self.push("settings saved"),
                Err(e) => self.push(format!("config write failed: {e:#}")),
            }
        }
        ui.add_space(6.0);
        ui.label("Config file:");
        match config_path() {
            Ok(p) => {
                ui.monospace(p.display().to_string());
            }
            Err(e) => {
                ui.weak(format!("unknown: {e}"));
            }
        }
        if ui
            .button("Open config folder")
            .on_hover_text("Open the folder that holds config.json")
            .clicked()
            && let Ok(p) = config_path()
            && let Some(dir) = p.parent()
        {
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("explorer").arg(dir).spawn();
        }
        #[cfg(target_os = "linux")]
        self.ui_keybinds(ui);
        #[cfg(target_os = "windows")]
        self.ui_hotkeys(ui);
        ui.add_space(10.0);
        ui.heading("About");
        ui.label(format!(
            "little_oil v{} — PoE automation",
            env!("CARGO_PKG_VERSION")
        ));
    }

    /// Linux-only: ready-to-paste WM keybinds for the detected desktop. The
    /// snippet names this executable, so the copied lines work as-is.
    #[cfg(target_os = "linux")]
    fn ui_keybinds(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.separator();
        ui.heading("Keybinds");
        let desktop = self.app.desktop();
        ui.label(crate::keybinds::instructions(desktop));
        let snippet = crate::keybinds::snippet(desktop, &crate::keybinds::exe_path());
        if ui
            .button("Copy keybinds")
            .on_hover_text("Copy the snippet to the clipboard")
            .clicked()
        {
            ui.ctx().copy_text(snippet.clone());
            self.push("keybind snippet copied");
        }
        ui.add_space(4.0);
        ui.monospace(&snippet);
    }

    /// Windows-only: one editable accelerator per registry action. Saving
    /// rewrites `settings.hotkeys`; `ensure_hotkeys` re-registers on the next
    /// frame because the binding list changed.
    #[cfg(target_os = "windows")]
    fn ui_hotkeys(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.separator();
        ui.heading("Hotkeys");
        ui.label(
            "Global shortcuts while Little Oil runs (e.g. F11, ctrl+shift+E). \
             Clear a field to unbind it.",
        );
        let mut map = self.app.settings.read().hotkeys.clone().unwrap_or_default();
        let mut dirty = false;
        egui::Grid::new("hotkeys_grid")
            .num_columns(2)
            .show(ui, |ui| {
                for def in ACTIONS {
                    ui.label(def.label);
                    let mut value = map
                        .get(def.id)
                        .cloned()
                        .or_else(|| def.default_hotkey.map(str::to_string))
                        .unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut value)
                                .desired_width(150.0)
                                .hint_text("unbound"),
                        )
                        .changed()
                    {
                        let value = value.trim().to_string();
                        map.insert(def.id.to_string(), value);
                        dirty = true;
                    }
                    ui.end_row();
                }
            });
        if dirty {
            self.app.settings.write().hotkeys = if map.is_empty() { None } else { Some(map) };
            match self.app.save_settings() {
                Ok(()) => self.push("hotkeys saved"),
                Err(e) => self.push(format!("config write failed: {e:#}")),
            }
        }
    }
}

/// Largest scale that fits `size` inside `avail` without upscaling.
const PREVIEW_AVAIL: egui::Vec2 = egui::Vec2::new(880.0, 560.0);

fn fit_into(size: egui::Vec2, avail: egui::Vec2) -> egui::Vec2 {
    let scale = (avail.x / size.x).min(avail.y / size.y).min(1.0);
    size * scale
}

#[cfg(test)]
mod actions_tab_tests {
    use super::*;
    use crate::test_support::test_app;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    struct TestState {
        app: Arc<App>,
        log: Arc<Mutex<GuiState>>,
        runner: Option<ActionRunner>,
    }

    /// The Actions tab is generated from the registry: one button per entry,
    /// and pressing one starts that action (its `▶ <id>…` line reaches the
    /// shared log).
    #[test]
    fn actions_tab_has_a_button_per_registry_action_and_runs_empty() {
        let (app, _fake) = test_app(crate::default_settings());
        let log = Arc::new(Mutex::new(GuiState {
            log: VecDeque::new(),
        }));
        let state = TestState {
            app: Arc::new(app),
            log: Arc::clone(&log),
            runner: None,
        };

        let mut harness = Harness::new_ui_state(
            |ui, st: &mut TestState| {
                if st.runner.is_none() {
                    st.runner = Some(ActionRunner {
                        app: Arc::clone(&st.app),
                        state: Arc::clone(&st.log),
                        busy: Arc::new(AtomicBool::new(false)),
                        ctx: ui.ctx().clone(),
                    });
                }
                let runner = st
                    .runner
                    .as_ref()
                    .expect("runner initialized on the first frame")
                    .clone();
                action_buttons(ui, false, |id, run| runner.run(id, run));
            },
            state,
        );

        // `get_by_label` panics when the label is missing, so this asserts a
        // button per registry entry.
        for def in ACTIONS {
            harness.get_by_label(def.label);
        }

        harness.get_by_label("Empty inventory (left)").click();
        harness.run();

        let log = log.lock();
        assert!(
            log.log.iter().any(|line| line.starts_with("▶ empty")),
            "clicking the empty button must start the empty action, log: {:?}",
            log.log
        );
    }
}

impl eframe::App for LittleOilGui {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Idempotent; installs the tray/hotkey event handlers once they exist.
        #[cfg(target_os = "windows")]
        {
            self.ensure_tray();
            self.ensure_hotkeys();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.tab != self.last_tab {
            self.last_tab = self.tab;
            self.health_checks = None; // re-run on next visit to Health
        }
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Little Oil");
                for (tab, hint) in [
                    ("Actions", "Run the inventory macros"),
                    (
                        "Setup",
                        "Guided first-run setup — detect window, regions, colors",
                    ),
                    (
                        "Calibrate",
                        "Drag-select regions; right-click a preview for whole window/monitor",
                    ),
                    ("Inventory", "Preview the inventory overlay"),
                    (
                        "Health",
                        "Diagnose why something isn't working (checkhealth-style)",
                    ),
                    ("Settings", "Tune behavior and find the config file"),
                ] {
                    ui.selectable_value(&mut self.tab, tab, tab)
                        .on_hover_text(hint);
                }
                ui.separator();
                ui.weak(&self.status);
            });
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            "Actions" => self.ui_actions(ui),
            "Setup" => self.ui_setup(ui, &ctx),
            "Calibrate" => self.ui_calibrate(ui, &ctx),
            "Inventory" => self.ui_inventory(ui, &ctx),
            "Health" => self.ui_health(ui),
            _ => self.ui_settings(ui),
        });
    }
}

// ── tray (Windows only) ──────────────────────────────────────────
// The tray lives on Windows. Linux users have WM keybinds + the panel, and
// tray-icon would drag the whole GTK stack in for nothing.

#[cfg(target_os = "windows")]
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};

/// Tray menu: one item per registry action, then Show/Quit.
#[cfg(target_os = "windows")]
fn build_menu() -> Result<Menu> {
    let menu = Menu::new();
    for def in ACTIONS {
        menu.append(&MenuItem::with_id(
            MenuId::new(def.id),
            def.label,
            true,
            None,
        ))?;
    }
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id(
        MenuId::new("show"),
        "Show window",
        true,
        None,
    ))?;
    menu.append(&PredefinedMenuItem::quit(None))?;
    Ok(menu)
}

#[cfg(target_os = "windows")]
fn tray_icon_data() -> Result<tray_icon::Icon> {
    // 32x32: dark rounded square with a lighter droplet.
    let size = 32usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let inside = (2..30).contains(&x) && (2..30).contains(&y);
            let droplet = {
                let dx = x as f32 - 16.0;
                let dy = y as f32 - 13.0;
                (dx * dx + dy * dy) < 72.0 && y > 8
            };
            let idx = (y * size + x) * 4;
            if droplet {
                rgba[idx..idx + 4].copy_from_slice(&[70, 190, 240, 255]);
            } else if inside {
                rgba[idx..idx + 4].copy_from_slice(&[30, 34, 42, 255]);
            } else {
                rgba[idx..idx + 4].copy_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Ok(tray_icon::Icon::from_rgba(rgba, size as u32, size as u32)?)
}

#[cfg(target_os = "windows")]
impl LittleOilGui {
    fn ensure_tray(&mut self) {
        if self.tray.is_some() {
            return;
        }
        // Menu clicks start actions directly from muda's handler, which runs in
        // its hidden window's WndProc on this thread's message pump — so they
        // work while the panel is idle or hidden. One-shot: later calls (tray
        // creation retries) are ignored.
        let runner = self.runner.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == "show" {
                runner
                    .ctx
                    .send_viewport_cmd(egui::ViewportCommand::Visible(true));
                runner.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                return;
            }
            if let Some(def) = ACTIONS.iter().find(|def| event.id == def.id) {
                runner.run(def.id, def.run);
            }
        }));
        let menu = match build_menu() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(?e, "tray menu build failed");
                return;
            }
        };
        let icon = match tray_icon_data() {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(?e, "tray icon build failed");
                return;
            }
        };
        match tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("Little Oil")
            .build()
        {
            Ok(t) => {
                tracing::info!("tray icon created");
                self.tray = Some(t);
            }
            Err(e) => tracing::warn!(?e, "tray icon creation failed (no tray host?)"),
        }
    }

    /// Register the global hotkeys: `settings.hotkeys` overrides, registry
    /// defaults otherwise. Re-registers (dropping the old manager) whenever the
    /// binding list changes, so saving the Settings tab takes effect at once.
    fn ensure_hotkeys(&mut self) {
        use global_hotkey::hotkey::HotKey;
        use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
        use std::str::FromStr;

        // Desired accelerators in registry order (first wins on duplicates).
        let mut desired: Vec<(&'static str, String)> = Vec::new();
        let mut unknown: Vec<String> = Vec::new();
        {
            let settings = self.app.settings.read();
            if let Some(map) = &settings.hotkeys {
                for id in map.keys() {
                    if ACTIONS.iter().all(|def| def.id != id) {
                        unknown.push(id.clone());
                    }
                }
            }
            for def in ACTIONS {
                let configured = settings
                    .hotkeys
                    .as_ref()
                    .and_then(|map| map.get(def.id))
                    .map(|s| s.trim().to_string());
                let accel = match configured {
                    // An explicitly cleared field unbinds the action.
                    Some(s) if s.is_empty() => continue,
                    Some(s) => s,
                    None => match def.default_hotkey {
                        Some(default) => default.to_string(),
                        None => continue,
                    },
                };
                desired.push((def.id, accel));
            }
        }
        let spec = format!(
            "{}|unknown={}",
            desired
                .iter()
                .map(|(id, accel)| format!("{id}={accel}"))
                .collect::<Vec<_>>()
                .join(","),
            unknown.join(",")
        );
        if self.hotkeys.is_some() && self.hotkey_spec == spec {
            return;
        }

        // Drop the old manager first so its registrations are released.
        self.hotkeys = None;
        self.hotkey_spec = spec;
        for id in &unknown {
            tracing::warn!("hotkey {id}: unknown action id");
        }

        let Ok(manager) = GlobalHotKeyManager::new() else {
            tracing::warn!("global hotkey manager unavailable");
            return;
        };
        let mut registered: Vec<(u32, &'static str, Action)> = Vec::new();
        let mut used: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for (id, accel) in &desired {
            let hotkey = match HotKey::from_str(accel) {
                Ok(hotkey) => hotkey,
                Err(e) => {
                    tracing::warn!("hotkey {id}: {e}");
                    continue;
                }
            };
            if !used.insert(hotkey.id()) {
                tracing::warn!("hotkey {id}: {accel:?} is already bound to another action");
                continue;
            }
            let Some(def) = ACTIONS.iter().find(|def| def.id == *id) else {
                continue;
            };
            if manager.register(hotkey).is_err() {
                tracing::warn!("hotkey {id}: the system refused to register {accel:?}");
                continue;
            }
            registered.push((hotkey.id(), def.id, def.run));
        }
        tracing::info!(count = registered.len(), "registered global hotkeys");
        // Like the tray handler, this runs from WM_HOTKEY in the manager's
        // hidden window, independent of egui repaints. No event can arrive
        // before it is installed: WM_HOTKEY is only delivered when the message
        // loop next pumps. Only key presses act — releases used to double-fire.
        let runner = self.runner.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state != HotKeyState::Pressed {
                return;
            }
            if let Some((_, label, f)) = registered.iter().find(|(id, _, _)| *id == event.id) {
                runner.run(label, *f);
            }
        }));
        self.hotkeys = Some(Hotkeys { _manager: manager });
    }
}

#[cfg(target_os = "windows")]
struct Hotkeys {
    _manager: global_hotkey::GlobalHotKeyManager,
}

// ── entry ────────────────────────────────────────────────────────

/// Run the GUI until the window is closed. Owns the eframe event loop.
pub(crate) fn run(app: Arc<App>) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([920.0, 660.0])
            .with_title("Little Oil"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Little Oil",
        options,
        Box::new(move |cc| Ok(Box::new(LittleOilGui::new(cc, app)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"))
}

// ── region picker (set-region, calibrate-point) ──────────────────

/// Single-purpose window: shows a fresh desktop capture, lets the user drag a
/// box (or right-click a whole window/monitor), and closes with the chosen
/// screen region. Enter or "Save selection" accepts; Esc or the close button
/// cancels.
struct RegionPicker<'a> {
    app: &'a App,
    prompt: &'a str,
    /// `Some(region)` once the user picked one; `None` means cancelled.
    outcome: Rc<RefCell<Option<ScreenRegion>>>,
    preview: Option<Preview>,
    /// Desktop capture taken before the window existed; turned into a texture
    /// on the first frame (egui only accepts large textures once the renderer
    /// is initialized).
    capture: Option<ScreenshotData>,
    drag: Option<(egui::Pos2, egui::Pos2)>,
    right_click_screen: Option<(i32, i32)>,
    status: String,
}

impl<'a> RegionPicker<'a> {
    /// `data` is the desktop capture taken *before* this window was created,
    /// so the picker shows the screen as it was and the window itself is not
    /// in the image.
    fn new(
        app: &'a App,
        prompt: &'a str,
        outcome: Rc<RefCell<Option<ScreenRegion>>>,
        data: ScreenshotData,
    ) -> Self {
        Self {
            app,
            prompt,
            outcome,
            preview: None,
            capture: Some(data),
            drag: None,
            right_click_screen: None,
            status: String::new(),
        }
    }

    /// Turn the stored capture into a texture. Runs on the first frame, when
    /// egui knows the renderer's maximum texture size (a multi-monitor desktop
    /// capture is larger than the 2048 px default available at window
    /// creation).
    fn ensure_preview(&mut self, ctx: &egui::Context) {
        let Some(data) = self.capture.take() else {
            return;
        };
        if data.width == 0 || data.height == 0 {
            self.status = "desktop capture returned an empty image".into();
            return;
        }
        let size = egui::Vec2::new(data.width as f32, data.height as f32);
        let color =
            egui::ColorImage::from_rgba_unmultiplied([data.width, data.height], &data.pixels);
        let texture = ctx.load_texture("region-picker", color, egui::TextureOptions::NEAREST);
        self.status = format!(
            "captured desktop {}x{} (origin {}, {})",
            data.width, data.height, data.origin.0, data.origin.1
        );
        self.preview = Some(Preview {
            data,
            texture,
            shown: size,
            rect: egui::Rect::ZERO,
        });
    }

    fn finish(&mut self, region: ScreenRegion, ctx: &egui::Context) {
        *self.outcome.borrow_mut() = Some(region);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn cancel(&mut self, ctx: &egui::Context) {
        *self.outcome.borrow_mut() = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// The screen region for the current drag, if any.
    fn drag_region(&self) -> Option<ScreenRegion> {
        let preview = self.preview.as_ref()?;
        let (a, b) = self.drag?;
        let rect = preview.rect;
        preview.geometry().region_from_drag(
            (a.x - rect.min.x, a.y - rect.min.y),
            (b.x - rect.min.x, b.y - rect.min.y),
        )
    }

    fn confirm(&mut self, ctx: &egui::Context) {
        match self.drag_region() {
            Some(region) => self.finish(region, ctx),
            None => {
                self.status = "drag a box first (or right-click for a whole window/monitor)".into();
            }
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(preview) = self.preview.as_ref() else {
            ui.weak(&self.status);
            return;
        };
        let image_size = egui::Vec2::new(preview.data.width as f32, preview.data.height as f32);
        let texture = preview.texture.id();
        let shown = fit_into(image_size, ui.available_size());
        let (rect, response) = ui.allocate_exact_size(shown, egui::Sense::click_and_drag());
        if let Some(preview) = self.preview.as_mut() {
            preview.shown = shown;
            preview.rect = rect;
        }
        ui.painter().image(
            texture,
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        if response.drag_started()
            && let Some(anchor) = ui.input(|i| i.pointer.press_origin())
        {
            // Anchor on the press position, not the point where egui decides
            // the press became a drag (that is already past the threshold, so
            // a fast drag would start from the wrong corner).
            let cur = response.interact_pointer_pos().unwrap_or(anchor);
            self.drag = Some((anchor, cur));
        }
        if response.dragged()
            && let (Some(drag), Some(cur)) = (self.drag.as_mut(), response.interact_pointer_pos())
        {
            drag.1 = cur;
        }
        if response.drag_stopped()
            && let Some(cur) = response.interact_pointer_pos()
        {
            let anchor = self.drag.unwrap_or((cur, cur)).0;
            self.drag = Some((anchor, cur));
            self.status = "press Enter or \"Save selection\" to save this box".into();
        }
        if response.secondary_clicked() {
            self.right_click_screen = response.interact_pointer_pos().and_then(|p| {
                let preview = self.preview.as_ref()?;
                let rect = preview.rect;
                preview
                    .geometry()
                    .screen_point((p.x - rect.min.x, p.y - rect.min.y))
            });
        }
        // Right-click menu: select the whole window or monitor under the cursor.
        response.context_menu(|ui| {
            if ui.button("Select whole window under cursor").clicked() {
                if let Some((sx, sy)) = self.right_click_screen {
                    match self.app.backend().window_at(sx, sy) {
                        Some(region) => self.finish(region, ctx),
                        None => self.status = "no window found under the cursor".into(),
                    }
                }
                ui.close();
            }
            if ui.button("Select whole monitor under cursor").clicked() {
                if let Some((sx, sy)) = self.right_click_screen {
                    match self.app.backend().monitor_at(sx, sy) {
                        Some(region) => self.finish(region, ctx),
                        None => self.status = "no monitor found under the cursor".into(),
                    }
                }
                ui.close();
            }
        });
        if let Some((a, b)) = self.drag {
            let r = egui::Rect::from_two_pos(a, b);
            ui.painter().rect_stroke(
                r,
                0.0,
                egui::Stroke::new(2.0_f32, egui::Color32::LIGHT_BLUE),
                egui::StrokeKind::Outside,
            );
            if let Some(region) = self.drag_region() {
                ui.painter().text(
                    r.max + egui::vec2(4.0, 0.0),
                    egui::Align2::LEFT_TOP,
                    format!(
                        "{}x{} at ({}, {})",
                        region.width, region.height, region.x, region.y
                    ),
                    egui::FontId::monospace(12.0),
                    egui::Color32::LIGHT_BLUE,
                );
            }
        }
    }
}

impl eframe::App for RegionPicker<'_> {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ensure_preview(&ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel(&ctx);
            return;
        }
        egui::Panel::top("picker-bar").show(ui, |ui| {
            ui.label(self.prompt);
            ui.horizontal(|ui| {
                if ui.button("Save selection").clicked() {
                    self.confirm(&ctx);
                }
                if ui.button("Cancel").clicked() {
                    self.cancel(&ctx);
                }
                ui.weak(&self.status);
            });
            ui.weak("Drag a box around the region. Enter saves, Esc cancels.");
        });
        egui::CentralPanel::default().show(ui, |ui| self.draw(ui, &ctx));
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.confirm(&ctx);
        }
    }
}

/// Ask the user for a screen region in the GUI picker.
///
/// `name` names the region in the window title (`Little Oil — select <name>`);
/// `prompt` is the instruction shown above the image. Closing the window or
/// pressing Esc cancels with `region selection cancelled`.
pub(crate) fn run_region_picker(app: &App, name: &str, prompt: &str) -> Result<ScreenRegion> {
    // Capture before the window exists so the picker never shows itself.
    let capture = app.backend().capture_desktop().map_err(|e| {
        anyhow::anyhow!("desktop capture failed — cannot open the region picker: {e:#}")
    })?;
    let title = format!("Little Oil — select {name}");
    let outcome: Rc<RefCell<Option<ScreenRegion>>> = Rc::new(RefCell::new(None));
    let shared = Rc::clone(&outcome);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_maximized(true)
            .with_title(title.clone()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(move |_cc| Ok(Box::new(RegionPicker::new(app, prompt, shared, capture)))),
    )
    .map_err(|e| anyhow::anyhow!("could not open the region picker: {e}"))?;
    let picked = *outcome.borrow();
    picked.ok_or_else(|| anyhow::anyhow!("region selection cancelled"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom(image: (u32, u32), origin: (i32, i32), shown: (f32, f32)) -> PreviewGeometry {
        PreviewGeometry::new(image, origin, egui::vec2(shown.0, shown.1))
    }

    fn region(r: ScreenRegion) -> (u32, u32, u32, u32) {
        (r.x, r.y, r.width, r.height)
    }

    #[test]
    fn drag_at_native_scale_maps_to_screen_coordinates() {
        let g = geom((1920, 1080), (0, 0), (1920.0, 1080.0));
        let r = g
            .region_from_drag((100.0, 50.0), (300.0, 150.0))
            .expect("drag inside the image");
        assert_eq!(region(r), (100, 50, 200, 100));
    }

    #[test]
    fn drag_in_a_shrunk_preview_maps_back_to_pixels() {
        // 1920x1080 capture drawn at half size (2 image pixels per point).
        let g = geom((1920, 1080), (0, 0), (960.0, 540.0));
        let r = g
            .region_from_drag((100.0, 25.0), (300.0, 75.0))
            .expect("drag inside the image");
        assert_eq!(region(r), (200, 50, 400, 100));
    }

    #[test]
    fn fractional_scale_and_negative_origin_clamp_to_zero() {
        // 1280x720 capture drawn at 1024x576 (0.8 points per pixel) from a
        // monitor left of and below the primary one.
        let g = geom((1280, 720), (-1920, 40), (1024.0, 576.0));
        let r = g
            .region_from_drag((80.0, 40.0), (400.0, 240.0))
            .expect("drag inside the image");
        // image (100, 50)..(500, 300) → x = -1820 clamped to 0, y = 40 + 50.
        assert_eq!(region(r), (0, 90, 400, 250));
    }

    #[test]
    fn positive_monitor_origin_is_added() {
        let g = geom((1280, 720), (1920, 0), (1280.0, 720.0));
        let r = g
            .region_from_drag((10.0, 20.0), (110.0, 120.0))
            .expect("drag inside the image");
        assert_eq!(region(r), (1930, 20, 100, 100));
    }

    #[test]
    fn reversed_drag_is_normalized() {
        let g = geom((1920, 1080), (0, 0), (1920.0, 1080.0));
        let forward = g.region_from_drag((100.0, 50.0), (300.0, 150.0));
        let reversed = g.region_from_drag((300.0, 150.0), (100.0, 50.0));
        assert_eq!(
            forward.map(region),
            reversed.map(region),
            "drag direction must not matter"
        );
    }

    #[test]
    fn zero_length_drag_is_one_pixel() {
        let g = geom((100, 100), (0, 0), (100.0, 100.0));
        let r = g
            .region_from_drag((10.0, 10.0), (10.0, 10.0))
            .expect("a click inside the image");
        assert_eq!(region(r), (10, 10, 1, 1));
    }

    #[test]
    fn drag_outside_the_image_is_rejected() {
        let g = geom((100, 100), (0, 0), (100.0, 100.0));
        assert!(g.region_from_drag((50.0, 50.0), (101.0, 60.0)).is_none());
        assert!(g.region_from_drag((-1.0, 0.0), (10.0, 10.0)).is_none());
    }

    #[test]
    fn empty_capture_has_no_geometry() {
        let g = geom((0, 0), (0, 0), (0.0, 0.0));
        assert!(g.region_from_drag((0.0, 0.0), (1.0, 1.0)).is_none());
        assert!(g.image_point((0.0, 0.0)).is_none());
    }

    #[test]
    fn screen_point_uses_the_capture_origin() {
        let g = geom((1920, 1080), (1920, -200), (960.0, 540.0));
        assert_eq!(g.screen_point((0.0, 0.0)), Some((1920, -200)));
        assert_eq!(
            g.screen_point((480.0, 270.0)),
            Some((1920 + 960, -200 + 540))
        );
        assert!(g.screen_point((-5.0, 0.0)).is_none());
    }
}
