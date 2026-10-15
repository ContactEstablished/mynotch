#![windows_subsystem = "windows"]

mod app;
mod desktop;
mod gfx;
mod icon;
mod media;
mod settings;
mod stats;
mod timer;
mod widgets;

fn main() -> windows::core::Result<()> {
    app::run()
}
