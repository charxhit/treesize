mod state;
mod ui;

use eframe::egui;
use state::AppState;

struct MyApp {
    state: AppState,
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            state: AppState::new(),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ui::draw(&mut self.state, ctx);
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "TreeSize RS",
        options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}
