#![windows_subsystem = "windows"]

mod app;
mod apps;
mod backdrop;
mod clipboard;
mod desktop;
mod gfx;
mod hotkey;
mod icon;
mod imaging;
mod media;
mod motion;
mod settings;
mod settings_panel;
mod stats;
mod text;
mod theme;
mod timer;
mod weather;
mod widgets;

fn main() -> windows::core::Result<()> {
    app::run()
}
