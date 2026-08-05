//! `little_oil gui` — tray icon (Windows) + settings/calibration panel.
//!
//! The panel is an eframe/egui window. Long-running macro actions run on
//! background threads; results stream into a shared log. On Windows the tray
//! menu and global hotkeys (F10/F11/F12) trigger the same actions; on Linux
//! the WM keybinds already do that, so no tray/hotkey is registered there.
//!
//! Tabs:
//! * **Actions** — empty inventory (left/right), recalibrate colors, log.
//! * **Calibrate** — capture the game window, drag a rectangle, save it as a
//!   region (game/inventory/stash/map), re-sample inventory colors.
//! * **Inventory** — capture + live occupied-slot overlay.
//! * **Settings** — edit region numbers, pointer scale, focus clicks.

use crate::app::App;
use crate::screenshot::ScreenshotData;
use crate::{ScreenRegion, config_path, save_config};
use anyhow::Result;
use eframe::egui;
use parking_lot::Mutex;
use std::collections::VecDeque;
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
}
struct LittleOilGui {
    app: Arc<App>,
    state: Arc<Mutex<GuiState>>,
    busy: Arc<AtomicBool>,

    tab: &'static str,
    preview: Option<Preview>,
    /// Drag rectangle in *display* space (shown image coords), when active.
    drag: Option<(egui::Pos2, egui::Pos2)>,
    region_target: RegionTarget,
    show_inv_overlay: bool,
    status: String,

    // Windows-only: system tray + global hotkeys. Kept here (the event-loop
    // thread) because both are !Send/!Sync on Windows.
    #[cfg(target_os = "windows")]
    tray: Option<tray_icon::TrayIcon>,
    #[cfg(target_os = "windows")]
    hotkeys: Option<Hotkeys>,
}

impl LittleOilGui {
    fn new(_cc: &eframe::CreationContext<'_>, app: Arc<App>) -> Self {
        let state = Arc::new(Mutex::new(GuiState {
            log: VecDeque::new(),
        }));
        let mut this = LittleOilGui {
            app,
            state,
            busy: Arc::new(AtomicBool::new(false)),
            tab: "Actions",
            preview: None,
            drag: None,
            region_target: RegionTarget::Game,
            show_inv_overlay: false,
            status: String::new(),
            #[cfg(target_os = "windows")]
            tray: None,
            #[cfg(target_os = "windows")]
            hotkeys: None,
        };
        this.status = this.summary_line();
        this
    }

    fn summary_line(&self) -> String {
        let s = self.app.settings.read();
        let plats = s
            .platform
            .map(|p| format!("{p:?}"))
            .unwrap_or_else(|| "auto".into());
        let inv = s
            .inv_region
            .map(|r| format!("{}x{}", r.width, r.height))
            .unwrap_or_else(|| "not set".into());
        format!(
            "platform: {plats} · inventory: {inv} · samples: {}",
            s.inv_samples.as_ref().map(|v| v.len()).unwrap_or(0)
        )
    }

    fn push(&mut self, msg: impl Into<String>) {
        self.state.lock().push(msg);
    }

    /// Run a macro action on a background thread, guarding against overlap.
    fn run_action(&mut self, name: &'static str, f: fn(&App) -> Result<()>) {
        if self.busy.load(Ordering::SeqCst) {
            self.push(format!("{name}: already busy — wait for the current action"));
            return;
        }
        self.busy.store(true, Ordering::SeqCst);
        let app = Arc::clone(&self.app);
        let state = Arc::clone(&self.state);
        let busy = Arc::clone(&self.busy);
        self.push(format!("▶ {name}…"));
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = f(&app);
            let elapsed = started.elapsed();
            let mut st = state.lock();
            match result {
                Ok(()) => st.push(format!("✓ {name} done in {:.1}s", elapsed.as_secs_f32())),
                Err(e) => st.push(format!("✗ {name} failed: {e:#}")),
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    fn capture_preview(&mut self, ctx: &egui::Context) {
        let settings = self.app.settings.read().clone();
        match settings.screenshot() {
            Ok(data) => {
                let (w, h) = (data.width, data.height);
                let (ox, oy) = data.origin;
                let size = egui::Vec2::new(w as f32, h as f32);
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    &data.pixels,
                );
                let texture = ctx.load_texture("capture", color, egui::TextureOptions::NEAREST);
                let shown = fit_into(size, PREVIEW_AVAIL);
                self.preview = Some(Preview { data, texture, shown });
                self.push(format!("captured {w}x{h} (origin {ox}, {oy})"));
            }
            Err(e) => self.push(format!("capture failed: {e:#}")),
        }
    }

    /// Convert a drag rect in display space to a screen-space region.
    fn drag_to_region(&self, a: egui::Pos2, b: egui::Pos2) -> Option<ScreenRegion> {
        let preview = self.preview.as_ref()?;
        let shown = fit_into(
            egui::Vec2::new(preview.data.width as f32, preview.data.height as f32),
            PREVIEW_AVAIL,
        );
        let origin_disp = egui::pos2((PREVIEW_AVAIL.x - shown.x) / 2.0, (PREVIEW_AVAIL.y - shown.y) / 2.0);
        let to_img = |p: egui::Pos2| -> Option<(u32, u32)> {
            let px = (p.x - origin_disp.x) / shown.x * preview.data.width as f32;
            let py = (p.y - origin_disp.y) / shown.y * preview.data.height as f32;
            if px < 0.0 || py < 0.0 || px > preview.data.width as f32 || py > preview.data.height as f32 {
                return None;
            }
            Some((px as u32, py as u32))
        };
        let (x0, y0) = to_img(a.min(b))?;
        let (x1, y1) = to_img(a.max(b))?;
        let w = x1.saturating_sub(x0).max(1);
        let h = y1.saturating_sub(y0).max(1);
        Some(ScreenRegion {
            x: preview.data.origin.0 + x0,
            y: preview.data.origin.1 + y0,
            width: w,
            height: h,
        })
    }

    fn save_dragged_region(&mut self) {
        let Some((a, b)) = self.drag else { return };
        self.drag = None;
        let Some(region) = self.drag_to_region(a, b) else {
            self.push("drag was outside the captured image — try again");
            return;
        };
        let target = self.region_target;
        {
            let mut s = self.app.settings.write();
            match target {
                RegionTarget::Game => s.game_window_region = Some(region),
                RegionTarget::Inventory => s.inv_region = Some(region),
                RegionTarget::Stash => s.stash_region = Some(region),
                RegionTarget::Map => s.map_region = Some(region),
            }
        }
        let settings_snapshot = self.app.settings.read().clone();
        let path = config_path().unwrap_or_else(|_| "config.json".into());
        match save_config(&path, &settings_snapshot) {
            Ok(()) => self.push(format!(
                "saved {} region: {}x{} at ({}, {})",
                target.label(), region.width, region.height, region.x, region.y
            )),
            Err(e) => self.push(format!("saved in memory but config write failed: {e:#}")),
        }
    }

    fn ui_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.busy.load(Ordering::SeqCst);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Empty inventory (left)"))
                .on_hover_text("Ctrl+click every occupied slot, up to 3 verify passes")
                .clicked()
            {
                self.run_action("empty", |app| app.empty_inv());
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Empty inventory (right)"))
                .on_hover_text("Same, right-click (emptyr)")
                .clicked()
            {
                self.run_action("emptyr", |app| app.empty_inv_right());
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Recalibrate inventory colors"))
                .on_hover_text("Re-sample the 60 empty-slot probe colors")
                .clicked()
            {
                self.run_action("recalibrate", |app| app.reset_inv_colors());
            }
        });
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
                let log = self.state.lock();
                for line in log.log.iter().rev().take(50).rev() {
                    ui.monospace(line);
                }
            });
    }

    fn ui_calibrate(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Calibrate");
        ui.horizontal(|ui| {
            if ui.button("Capture game window").clicked() {
                self.capture_preview(ctx);
            }
            ui.label("then drag a rectangle below and save it as:");
            egui::ComboBox::from_id_salt("region_target")
                .selected_text(self.region_target.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.region_target, RegionTarget::Game, RegionTarget::Game.label());
                    ui.selectable_value(&mut self.region_target, RegionTarget::Inventory, RegionTarget::Inventory.label());
                    ui.selectable_value(&mut self.region_target, RegionTarget::Stash, RegionTarget::Stash.label());
                    ui.selectable_value(&mut self.region_target, RegionTarget::Map, RegionTarget::Map.label());
                });
            if ui.button("Save selection").clicked() {
                self.save_dragged_region();
            }
        });
        ui.add_space(4.0);
        if let Some(prev) = &self.preview {
            let (rect, response) = ui.allocate_exact_size(prev.shown, egui::Sense::drag());
            ui.painter().image(
                prev.texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            if response.drag_started() {
                let pos = response.interact_pointer_pos().unwrap_or_default();
                self.drag = Some((pos, pos));
            }
            if response.dragged() {
                if let (Some(drag), Some(cur)) = (self.drag.as_mut(), response.interact_pointer_pos()) {
                    drag.1 = cur;
                }
            }
            if response.drag_stopped() {
                if let Some(cur) = response.interact_pointer_pos() {
                    let anchor = self.drag.unwrap_or((cur, cur)).0;
                    self.drag = Some((anchor, cur));
                }
                self.save_dragged_region();
            }
            // Draw the drag overlay.
            if let Some((a, b)) = self.drag {
                let r = egui::Rect::from_two_pos(a, b);
                ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(2.0_f32, egui::Color32::LIGHT_BLUE), egui::StrokeKind::Outside);
                ui.painter().text(
                    r.max + egui::vec2(4.0, 0.0),
                    egui::Align2::LEFT_TOP,
                    self.region_target.label(),
                    egui::FontId::monospace(12.0),
                    egui::Color32::LIGHT_BLUE,
                );
            }
        } else {
            ui.weak("No capture yet — click \"Capture game window\".");
        }
        ui.add_space(8.0);
        let s = self.app.settings.read();
        if let Some(r) = s.game_window_region {
            ui.label(format!("game: {}x{} @ ({}, {})", r.width, r.height, r.x, r.y));
        }
        if let Some(r) = s.inv_region {
            ui.label(format!("inventory: {}x{} @ ({}, {})", r.width, r.height, r.x, r.y));
        }
        if s.inv_samples.as_ref().map(|v| v.len()) != Some(60) {
            ui.colored_label(
                egui::Color32::from_rgb(220, 160, 60),
                "Inventory colors not sampled — run \"Recalibrate inventory colors\" after setting the inventory region.",
            );
        }
    }

    fn ui_inventory(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Inventory");
        ui.horizontal(|ui| {
            if ui.button("Capture").clicked() {
                self.capture_preview(ctx);
            }
            ui.checkbox(&mut self.show_inv_overlay, "overlay occupied slots");
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
        let Some(inv) = settings.inv_region else { return };
        let Some(samples) = settings.inv_samples.clone() else { return };
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
                                    .try_get_pixel(p.0 as usize, p.1 as usize)
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
                    ui.painter().rect_filled(cell, 0.0, egui::Color32::from_rgba_unmultiplied(220, 60, 60, 110));
                }
                ui.painter().rect_stroke(
                    cell,
                    0.0,
                    egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 60)),
                    egui::StrokeKind::Inside,
                );
        }
    }
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.label("Regions are in screen pixels. Calibrate them with the drag-select on the Calibrate tab instead of typing numbers.");
        ui.add_space(6.0);
        let mut dirty = false;
        {
            let mut s = self.app.settings.write();
            egui::Grid::new("settings_grid").num_columns(2).show(ui, |ui| {
                let mut focus_clicks = s.focus_clicks;
                ui.label("Focus clicks:");
                if ui.add(egui::DragValue::new(&mut focus_clicks).range(1..=5)).changed() {
                    s.focus_clicks = focus_clicks;
                    dirty = true;
                }
                ui.end_row();
                let mut scale = s.pointer_scale.unwrap_or(1.25);
                ui.label("Pointer scale:");
                if ui.add(egui::DragValue::new(&mut scale).speed(0.01).range(0.5..=3.0)).changed() {
                    s.pointer_scale = Some(scale);
                    dirty = true;
                }
                ui.end_row();
            });
        }
        if dirty {
            let settings_snapshot = self.app.settings.read().clone();
            let path = config_path().unwrap_or_else(|_| "config.json".into());
            match save_config(&path, &settings_snapshot) {
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
        if ui.button("Open config folder").clicked() {
            if let Ok(p) = config_path() {
                if let Some(dir) = p.parent() {
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
            }
        }
        ui.add_space(10.0);
        ui.heading("About");
        ui.label(format!("little_oil v{} — PoE automation", env!("CARGO_PKG_VERSION")));
    }
}

/// Largest scale that fits `size` inside `avail` without upscaling.
const PREVIEW_AVAIL: egui::Vec2 = egui::Vec2::new(880.0, 560.0);

fn fit_into(size: egui::Vec2, avail: egui::Vec2) -> egui::Vec2 {
    let scale = (avail.x / size.x).min(avail.y / size.y).min(1.0);
    size * scale
}

impl eframe::App for LittleOilGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "windows")]
        self.pump_tray_events();
        #[cfg(target_os = "windows")]
        self.pump_hotkey_events();
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Little Oil");
                for tab in ["Actions", "Calibrate", "Inventory", "Settings"] {
                    ui.selectable_value(&mut self.tab, tab, tab);
                }
                ui.separator();
                ui.weak(&self.status);
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            "Actions" => self.ui_actions(ui),
            "Calibrate" => self.ui_calibrate(ui, ctx),
            "Inventory" => self.ui_inventory(ui, ctx),
            _ => self.ui_settings(ui),
        });
    }
}

// ── tray (Windows only) ──────────────────────────────────────────
// The tray lives on Windows. Linux users have WM keybinds + the panel, and
// tray-icon would drag the whole GTK stack in for nothing.

#[cfg(target_os = "windows")]
fn build_menu() -> Result<muda::Menu> {
    let menu = muda::Menu::new();
    menu.append_items(&[
        &muda::MenuItem::with_id(muda::MenuId::new("empty"), "Empty inventory (left)", true, None),
        &muda::MenuItem::with_id(muda::MenuId::new("emptyr"), "Empty inventory (right)", true, None),
        &muda::MenuItem::with_id(muda::MenuId::new("recalibrate"), "Recalibrate inventory colors", true, None),
        &muda::PredefinedMenuItem::separator(),
        &muda::PredefinedMenuItem::quit(None),
    ])?;
    Ok(menu)
}

#[cfg(target_os = "windows")]
fn tray_icon_data() -> Result<tray_icon::Icon> {
    // 32x32: dark rounded square with a lighter droplet.
    let size = 32usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let inside = x >= 2 && x < 30 && y >= 2 && y < 30;
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

    fn pump_tray_events(&mut self) {
        self.ensure_tray();
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            let id = event.id().0.as_str();
            match id {
                "empty" => self.run_action("empty", |app| app.empty_inv()),
                "emptyr" => self.run_action("emptyr", |app| app.empty_inv_right()),
                "recalibrate" => self.run_action("recalibrate", |app| app.reset_inv_colors()),
                _ => {}
            }
        }
    }

    fn ensure_hotkeys(&mut self) {
        if self.hotkeys.is_some() {
            return;
        }
        use global_hotkey::hotkey::{Code, HotKey};
        use global_hotkey::GlobalHotKeyManager;
        let Ok(manager) = GlobalHotKeyManager::new() else {
            tracing::warn!("global hotkey manager unavailable");
            return;
        };
        let candidates = [
            (HotKey::new(None, Code::F10), "emptyr (F10)" as &'static str, "emptyr" as &'static str),
            (HotKey::new(None, Code::F11), "empty (F11)", "empty"),
            (HotKey::new(None, Code::F12), "recalibrate (F12)", "recalibrate"),
        ];
        let mut actions: Vec<(u32, &'static str, &'static str)> = Vec::new();
        for (hk, label, action) in candidates {
            if manager.register(hk).is_ok() {
                actions.push((hk.id(), label, action));
            }
        }
        tracing::info!(count = actions.len(), "registered global hotkeys (F10 emptyr, F11 empty, F12 recalibrate)");
        self.hotkeys = Some(Hotkeys {
            _manager: manager,
            actions,
        });
    }

    fn pump_hotkey_events(&mut self) {
        self.ensure_hotkeys();
        use global_hotkey::GlobalHotKeyEvent;
        // Collect matches under a short borrow, then run actions after it ends.
        let mut pending: Vec<(&'static str, &'static str)> = Vec::new();
        {
            let Some(hotkeys) = &self.hotkeys else { return };
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                let id = event.id();
                for (hk_id, label, action) in &hotkeys.actions {
                    if *hk_id == id {
                        pending.push((label, action));
                        break;
                    }
                }
            }
        }
        for (label, action) in pending {
            match action {
                "empty" => self.run_action(label, |app| app.empty_inv()),
                "emptyr" => self.run_action(label, |app| app.empty_inv_right()),
                _ => self.run_action(label, |app| app.reset_inv_colors()),
            }
        }
    }
}

#[cfg(target_os = "windows")]
struct Hotkeys {
    _manager: global_hotkey::GlobalHotKeyManager,
    /// (hotkey id, log label, action name)
    actions: Vec<(u32, &'static str, &'static str)>,
}

// ── entry ────────────────────────────────────────────────────────

/// Run the GUI until the window is closed. Owns the eframe event loop.
pub(crate) fn run(app: Arc<App>) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([920.0, 660.0])
            .with_title("Little Oil"),
        ..Default::default()
    };
    eframe::run_native(
        "Little Oil",
        options,
        Box::new(move |cc| Ok(Box::new(LittleOilGui::new(cc, app)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"))
}
