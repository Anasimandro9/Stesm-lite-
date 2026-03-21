use eframe::egui;
use eframe::egui::{Color32, FontId, RichText};

struct SteamLite;

impl eframe::App for SteamLite {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut style = (*ctx.style()).clone();
        style.visuals.dark_mode = true;
        style.visuals.panel_fill = Color32::from_rgb(15, 18, 25);
        ctx.set_style(style);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("🎮 STEAM LITE")
                            .font(FontId::proportional(48.0))
                            .color(Color32::from_rgb(100, 200, 255))
                            .strong(),
                    );
                    ui.add_space(16.0);
                    ui.label(
                        RichText::new("✅ Sistema 1 OK — Base funcionando")
                            .font(FontId::proportional(20.0))
                            .color(Color32::from_rgb(100, 220, 120)),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Siguiente: Sistema 2 — Auth & Config")
                            .font(FontId::proportional(15.0))
                            .color(Color32::from_rgb(140, 160, 180)),
                    );
                });
            });
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Steam Lite")
            .with_inner_size([900.0, 620.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Steam Lite",
        options,
        Box::new(|_cc| Box::new(SteamLite)),
    )
}
