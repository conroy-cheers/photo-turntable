use app::TurntableApp;
use eframe::NativeOptions;

mod app;
mod camera;
mod turntable;

fn main() -> Result<(), eframe::Error> {
    let native_options = NativeOptions::default();
    eframe::run_native(
        "Turntable Controller",
        native_options,
        Box::new(|cc| Ok(Box::new(TurntableApp::<turntable::RevoTurntable>::new(cc)))),
    )
}
