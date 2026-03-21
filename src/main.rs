use eframe::egui;
use eframe::egui::{Color32, FontId, RichText, Rounding, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

// ===================== CONFIG =====================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    api_key: String,
    steam_id: String,
}

fn config_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.pop();
    p.push("steamlite_config.json");
    p
}

fn load_config() -> Option<Config> {
    let data = fs::read_to_string(config_path()).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_config(config: &Config) {
    if let Ok(json) = serde_json::to_string_pretty(config) {
        fs::write(config_path(), json).ok();
    }
}

fn delete_config() {
    fs::remove_file(config_path()).ok();
}

// ===================== APP STATE =====================

#[derive(PartialEq)]
enum Screen {
    Setup,
    Main,
}

struct SteamLite {
    screen: Screen,
    config: Option<Config>,
    input_api_key: String,
    input_steam_id: String,
    show_api_key: bool,
    setup_error: String,
}

impl SteamLite {
    fn new(_cc: &eframe::CreationContext) -> Self {
        let config = load_config();
        let screen = if config.is_some() {
            Screen::Main
        } else {
            Screen::Setup
        };

        Self {
            screen,
            config,
            input_api_key: String::new(),
            input_steam_id: String::new(),
            show_api_key: false,
            setup_error: String::new(),
        }
    }

    fn validate_and_save(&mut self) {
        let api_key = self.input_api_key.trim().to_string();
        let steam_id = self.input_steam_id.trim().to_string();

        if api_key.is_empty() {
            self.setup_error = "Necesitas la API Key".to_string();
            return;
        }
        if api_key.len() < 20 {
            self.setup_error = "La API Key parece incorrecta (muy corta)".to_string();
            return;
        }
        if steam_id.is_empty() {
            self.setup_error = "Necesitas el Steam ID".to_string();
            return;
        }
        if !steam_id.starts_with("765611") || steam_id.len() != 17 {
            self.setup_error =
                "El Steam ID debe tener 17 digitos y empezar con 765611".to_string();
            return;
        }
        if steam_id.parse::<u64>().is_err() {
            self.setup_error = "El Steam ID solo debe contener numeros".to_string();
            return;
        }

        let config = Config { api_key, steam_id };
        save_config(&config);
        self.config = Some(config);
        self.setup_error = String::new();
        self.screen = Screen::Main;
    }
}

// ===================== PANTALLA SETUP =====================

impl SteamLite {
    fn show_setup(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(12, 15, 22)))
            .show(ctx, |ui| {
                ui.add_space(50.0);

                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🎮").font(FontId::proportional(64.0)));
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("STEAM LITE")
                            .font(FontId::proportional(38.0))
                            .color(Color32::from_rgb(100, 200, 255))
                            .strong(),
                    );
                    ui.label(
                        RichText::new("Cliente ligero para PCs con pocos recursos")
                            .font(FontId::proportional(14.0))
                            .color(Color32::from_rgb(120, 140, 165)),
                    );

                    ui.add_space(36.0);

                    egui::Frame::none()
                        .fill(Color32::from_rgb(20, 26, 38))
                        .rounding(Rounding::same(14.0))
                        .inner_margin(egui::Margin::same(32.0))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(40, 55, 80)))
                        .show(ui, |ui| {
                            ui.set_max_width(480.0);

                            ui.label(
                                RichText::new("Configuracion inicial")
                                    .font(FontId::proportional(20.0))
                                    .color(Color32::WHITE)
                                    .strong(),
                            );
                            ui.label(
                                RichText::new("Solo necesitas hacerlo una vez")
                                    .font(FontId::proportional(13.0))
                                    .color(Color32::from_rgb(100, 120, 150)),
                            );

                            ui.add_space(24.0);

                            // API Key
                            ui.label(
                                RichText::new("Steam API Key")
                                    .font(FontId::proportional(14.0))
                                    .color(Color32::from_rgb(180, 200, 230))
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.input_api_key)
                                        .desired_width(360.0)
                                        .hint_text("Ej: A1B2C3D4E5F6G7H8I9J0...")
                                        .password(!self.show_api_key),
                                );
                                let eye = if self.show_api_key { "Ocultar" } else { "Mostrar" };
                                if ui.small_button(eye).clicked() {
                                    self.show_api_key = !self.show_api_key;
                                }
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Obtenela en: ")
                                        .font(FontId::proportional(12.0))
                                        .color(Color32::from_rgb(100, 120, 150)),
                                );
                                ui.hyperlink_to(
                                    RichText::new("steamcommunity.com/dev/apikey")
                                        .font(FontId::proportional(12.0))
                                        .color(Color32::from_rgb(80, 160, 240)),
                                    "https://steamcommunity.com/dev/apikey",
                                );
                            });

                            ui.add_space(20.0);

                            // Steam ID
                            ui.label(
                                RichText::new("Steam ID (64-bit)")
                                    .font(FontId::proportional(14.0))
                                    .color(Color32::from_rgb(180, 200, 230))
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.input_steam_id)
                                    .desired_width(420.0)
                                    .hint_text("Ej: 76561198XXXXXXXXX (17 digitos)"),
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Encontralo en: ")
                                        .font(FontId::proportional(12.0))
                                        .color(Color32::from_rgb(100, 120, 150)),
                                );
                                ui.hyperlink_to(
                                    RichText::new("steamidfinder.com")
                                        .font(FontId::proportional(12.0))
                                        .color(Color32::from_rgb(80, 160, 240)),
                                    "https://steamidfinder.com",
                                );
                            });

                            ui.add_space(24.0);

                            // Error
                            if !self.setup_error.is_empty() {
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(60, 20, 20))
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "Error: {}",
                                                self.setup_error
                                            ))
                                            .font(FontId::proportional(13.0))
                                            .color(Color32::from_rgb(255, 120, 120)),
                                        );
                                    });
                                ui.add_space(12.0);
                            }

                            // Boton
                            let btn = ui.add_sized(
                                Vec2::new(420.0, 46.0),
                                egui::Button::new(
                                    RichText::new("ENTRAR")
                                        .font(FontId::proportional(17.0))
                                        .color(Color32::WHITE)
                                        .strong(),
                                )
                                .fill(Color32::from_rgb(30, 110, 200))
                                .rounding(Rounding::same(8.0)),
                            );

                            if btn.clicked() {
                                self.validate_and_save();
                            }

                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                self.validate_and_save();
                            }
                        });

                    ui.add_space(20.0);

                    egui::Frame::none()
                        .fill(Color32::from_rgb(15, 30, 20))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(20.0, 10.0))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(30, 80, 40)))
                        .show(ui, |ui| {
                            ui.set_max_width(480.0);
                            ui.label(
                                RichText::new(
                                    "Tus datos se guardan solo en tu PC. No se envian a ningun servidor externo.",
                                )
                                .font(FontId::proportional(12.0))
                                .color(Color32::from_rgb(100, 180, 120)),
                            );
                        });
                });
            });
    }
}

// ===================== PANTALLA MAIN (placeholder) =====================

impl SteamLite {
    fn show_main(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(10, 13, 20))
                    .inner_margin(egui::Margin::symmetric(16.0, 10.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("STEAM LITE")
                            .font(FontId::proportional(20.0))
                            .color(Color32::from_rgb(100, 200, 255))
                            .strong(),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Cerrar sesion").clicked() {
                            delete_config();
                            self.config = None;
                            self.input_api_key = String::new();
                            self.input_steam_id = String::new();
                            self.screen = Screen::Setup;
                        }

                        if let Some(cfg) = &self.config {
                            ui.label(
                                RichText::new(format!("ID: {}", &cfg.steam_id))
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::from_rgb(100, 130, 170)),
                            );
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(15, 18, 25)))
            .show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Sistema 2 OK - Auth funcionando")
                                .font(FontId::proportional(24.0))
                                .color(Color32::from_rgb(100, 220, 120))
                                .strong(),
                        );
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Config guardada correctamente en tu PC")
                                .font(FontId::proportional(15.0))
                                .color(Color32::from_rgb(140, 160, 180)),
                        );
                        if let Some(cfg) = &self.config {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(format!("Steam ID: {}", cfg.steam_id))
                                    .font(FontId::proportional(14.0))
                                    .color(Color32::from_rgb(100, 180, 255)),
                            );
                        }
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new("Siguiente: Sistema 3 - Biblioteca con portadas reales")
                                .font(FontId::proportional(14.0))
                                .color(Color32::from_rgb(100, 120, 150)),
                        );
                    });
                });
            });
    }
}

// ===================== APP LOOP =====================

impl eframe::App for SteamLite {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut style = (*ctx.style()).clone();
        style.visuals.dark_mode = true;
        style.visuals.panel_fill = Color32::from_rgb(15, 18, 25);
        style.visuals.widgets.inactive.rounding = Rounding::same(6.0);
        style.visuals.widgets.hovered.rounding = Rounding::same(6.0);
        style.visuals.widgets.active.rounding = Rounding::same(6.0);
        ctx.set_style(style);

        match self.screen {
            Screen::Setup => self.show_setup(ctx),
            Screen::Main => self.show_main(ctx),
        }
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
        Box::new(|cc| Box::new(SteamLite::new(cc))),
    )
}
