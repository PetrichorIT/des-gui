#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

// When compiling natively:
//
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use des_gui::launch_with_gui;
    launch_with_gui(des_gui::sim::sim)
}
