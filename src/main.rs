#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

// When compiling natively:
//
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let mut native_options = eframe::NativeOptions::default();
    native_options.viewport.maximized = Some(true);

    eframe::run_native(
        "des-gui",
        native_options,
        Box::new(|cc| Ok(Box::new(des_gui::Application::new(cc, des_gui::sim::sim)))),
    )
}
