mod app;
mod engine;
mod generator;
mod stats;
mod storage;

fn main() -> eframe::Result {
    engine::warm_up();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("FasterTypst")
            .with_app_id("fastertypst")
            .with_inner_size([1000.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native("FasterTypst", options, Box::new(|_cc| Ok(Box::new(app::App::new()))))
}
