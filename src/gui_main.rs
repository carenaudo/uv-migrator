#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use uv_migrator::gui::UvMigratorApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("⚡ uv-migrator - Python Environment Migrator")
            .with_inner_size(egui::vec2(920.0, 660.0))
            .with_min_inner_size(egui::vec2(760.0, 520.0)),
        ..Default::default()
    };

    eframe::run_native(
        "uv-migrator-gui",
        options,
        Box::new(|cc| Ok(Box::new(UvMigratorApp::new(cc)))),
    )
}
