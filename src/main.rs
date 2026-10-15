#![windows_subsystem = "windows"]

mod app;
mod gfx;
mod media;
mod stats;
mod widgets;

fn main() -> windows::core::Result<()> {
    app::run()
}
