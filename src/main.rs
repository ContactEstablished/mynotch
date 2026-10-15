#![windows_subsystem = "windows"]

mod app;
mod apps;
mod clipboard;
mod desktop;
mod gfx;
mod icon;
mod imaging;
mod media;
mod settings;
mod settings_panel;
mod stats;
mod timer;
mod widgets;

fn main() -> windows::core::Result<()> {
    app::run()
}
