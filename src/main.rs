#![windows_subsystem = "windows"]

mod app;
mod desktop;
mod gfx;
mod media;
mod settings;
mod stats;
mod widgets;

fn main() -> windows::core::Result<()> {
    app::run()
}
