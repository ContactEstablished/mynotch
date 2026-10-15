#![windows_subsystem = "windows"]

mod app;
mod stats;

fn main() -> windows::core::Result<()> {
    app::run()
}
