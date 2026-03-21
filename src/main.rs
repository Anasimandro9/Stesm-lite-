use eframe::egui;
use eframe::egui::{Color32, FontId, RichText, Rounding, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

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

// ===================== GAME =====================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Game {
    appid: u64,
    name: String,
    playtime_forever: u64,
}

impl Game {
    fn playtime_hours(&self) -> f32 {
        self.playtime_forever as f32 / 60.0
    }

    fn image_url(&self) -> String {
        format!(
            "https://cdn.cloudflare.steamstatic.com/steam/apps/{}/header.jpg",
            self.appid
        )
    }

    fn launch_url(&self) -> String {
        format!("steam://run/{}", self.appid)
    }

    fn store_url(&self) -> String {
        format!("https://store.steampowered.com/app/{}", self.appid)
    }
}

// ===================== API =====================

#[derive(Deserialize)]
struct GamesResponse {
    response: GamesResponseInner,
}

#[derive(Deserialize)]
struct GamesResponseInner {
    games: Option<Vec<Game>>,
}

fn fetch_games(api_key: &str, steam_id: &str) -> Result<Vec<Game>, String> {
    let url = format!(
        "https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?key={}&steamid={}&include_appinfo=true&include_played_free_games=true",
        api_key, steam_id
    );
    let resp = reqwest::blocking::get(&url).map_err(|e| e.to_string())?;
    let data: GamesResponse = resp.json().map_err(|e| e.to_string())?;
    let mut games = data.response.games.unwrap_or_default();
    games.sort_by(|a, b| b.playtime_forever.cmp(&a.playtime_forever));
    Ok(games)
}

fn fetch_image(url: &str) -> Option<Vec<u8>> {
    let resp = reqwest::blocking::get(url).ok()?;
    if resp.status().is_success() {
        resp.bytes().ok().map(|b| b.to_vec())
    } else {
        None
    }
}

// ===================== SCREENS =====================

#[derive(PartialEq)]
enum Screen {
    Setup,
    Main,
}

// ===================== APP =====================

struct SteamLite {
    screen: Screen,
    config: Option<Config>,

    // Setup
    input_api_key: String,
    input_steam_id: String,
    show_api_key: bool,
    setup_error: String,

    // Library
    games: Arc<Mutex<Vec<Game>>>,
    loading_games: Arc<Mutex<bool>>,
    load_error: Arc<Mutex<String>>,
    search: String,

    // Image cache: appid -> TextureHandle
    textures: HashMap<u64, egui::TextureHandle>,
    // Images being fetched
    pending_images: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
    fetching: Arc<Mutex<std::collections::HashSet<u64>>>,

    last_launched: Option<String>,
}

impl SteamLite {
    fn new(_cc: &eframe::CreationContext) -> Self {
        let config = load_config();
        let screen = if config.is_some() {
            Screen::Main
        } else {
            Screen::Setup
        };

        let app = Self {
            screen,
            config,
            input_api_key: String::new(),
            input_steam_id: String::new(),
            show_api_key: false,
            setup_error: String::new(),
            games: Arc::new(Mutex::new(vec![])),
            loading_games: Arc::new(Mutex::new(false)),
            load_error: Arc::new(Mutex::new(String::new())),
            search: String::new(),
            textures: HashMap::new(),
            pending_images: Arc::new(Mutex::new(vec![])),
            fetching: Arc::new(Mutex::new(std::collections::HashSet::new())),
            last_launched: None,
        };

        app
    }

    fn start_loading_games(&self) {
        let cfg = match &self.config {
            Some(c) => c.clone(),
            None => return,
        };

        let games = Arc::clone(&self.games);
        let loading = Arc::clone(&self.loading_games);
        let error = Arc::clone(&self.load_error);

        *loading.lock().unwrap() = true;
        *error.lock().unwrap() = String::new();

        thread::spawn(move || {
            match fetch_games(&cfg.api_key, &cfg.steam_id) {
                Ok(g) => *games.lock().unwrap() = g,
                Err(e) => *error.lock().unwrap() = format!("Error al cargar juegos: {}", e),
            }
            *loading.lock().unwrap() = false;
        });
    }

    fn request_image(&self, appid: u64, url: String) {
        let mut fetching = self.fetching.lock().unwrap();
        if fetching.contains(&appid) {
            return;
        }
        fetching.insert(appid);
        drop(fetching);

        let pending = Arc::clone(&self.pending_images);
        let fetching = Arc::clone(&self.fetching);

        thread::spawn(move || {
            if let Some(bytes) = fetch_image(&url) {
                pending.lock().unwrap().push((appid, bytes));
            }
            fetching.lock().unwrap().remove(&appid);
        });
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
            self.setup_error = "El Steam ID debe tener 17 digitos y empezar con 765611".to_string();
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
        self.start_loading_games();
    }
}

// ===================== SETUP SCREEN =====================

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

                            if !self.setup_error.is_empty() {
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(60, 20, 20))
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(format!("Error: {}", self.setup_error))
                                                .font(FontId::proportional(13.0))
                                                .color(Color32::from_rgb(255, 120, 120)),
                                        );
                                    });
                                ui.add_space(12.0);
                            }

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
                                RichText::new("Tus datos se guardan solo en tu PC. No se envian a ningun servidor externo.")
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::from_rgb(100, 180, 120)),
                            );
                        });
                });
            });
    }
}

// ===================== MAIN SCREEN =====================

impl SteamLite {
    fn show_main(&mut self, ctx: &egui::Context) {
        // Procesar imagenes descargadas
        {
            let mut pending = self.pending_images.lock().unwrap();
            for (appid, bytes) in pending.drain(..) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    let img = img.to_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    let pixels: Vec<egui::Color32> = img
                        .pixels()
                        .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
                        .collect();
                    let texture = ctx.load_texture(
                        format!("game_{}", appid),
                        egui::ColorImage { size, pixels },
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(appid, texture);
                }
            }
        }

        // Top bar
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

                    ui.add_space(20.0);

                    // Buscador
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .desired_width(250.0)
                            .hint_text("Buscar juego..."),
                    );

                    // Contador
                    let count = self.games.lock().unwrap().len();
                    let loading = *self.loading_games.lock().unwrap();
                    ui.add_space(10.0);
                    if loading {
                        ui.label(
                            RichText::new("Cargando biblioteca...")
                                .font(FontId::proportional(13.0))
                                .color(Color32::from_rgb(200, 180, 50)),
                        );
                    } else if count > 0 {
                        ui.label(
                            RichText::new(format!("{} juegos", count))
                                .font(FontId::proportional(13.0))
                                .color(Color32::from_rgb(120, 140, 170)),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Cerrar sesion").clicked() {
                            delete_config();
                            self.config = None;
                            self.input_api_key = String::new();
                            self.input_steam_id = String::new();
                            self.screen = Screen::Setup;
                            *self.games.lock().unwrap() = vec![];
                        }

                        if let Some(name) = &self.last_launched {
                            ui.label(
                                RichText::new(format!("Jugando: {}", name))
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::from_rgb(100, 220, 120)),
                            );
                            ui.add_space(10.0);
                        }
                    });
                });
            });

        // Error banner
        let error = self.load_error.lock().unwrap().clone();
        if !error.is_empty() {
            egui::TopBottomPanel::top("error_bar")
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgb(80, 20, 20))
                        .inner_margin(egui::Margin::symmetric(16.0, 8.0)),
                )
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new(&error)
                            .color(Color32::from_rgb(255, 150, 150))
                            .font(FontId::proportional(13.0)),
                    );
                });
        }

        // Library grid
        let games_snap: Vec<Game> = {
            let games = self.games.lock().unwrap();
            let q = self.search.to_lowercase();
            games
                .iter()
                .filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q))
                .cloned()
                .collect()
        };

        // Request images for visible games (first 30)
        for game in games_snap.iter().take(30) {
            if !self.textures.contains_key(&game.appid) {
                self.request_image(game.appid, game.image_url());
            }
        }

        let mut launch: Option<Game> = None;
        let mut open_store: Option<Game> = None;

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(13, 16, 23)))
            .show(ctx, |ui| {
                if games_snap.is_empty() && !*self.loading_games.lock().unwrap() && error.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("No se encontraron juegos")
                                .font(FontId::proportional(18.0))
                                .color(Color32::from_rgb(100, 120, 150)),
                        );
                    });
                    return;
                }

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(12.0);

                    let card_w = 220.0_f32;
                    let card_h = 160.0_f32; // image area
                    let spacing = 12.0_f32;
                    let available = ui.available_width() - 24.0;
                    let cols = ((available + spacing) / (card_w + spacing)).floor().max(1.0) as usize;

                    egui::Grid::new("library")
                        .num_columns(cols)
                        .spacing(Vec2::splat(spacing))
                        .min_col_width(card_w)
                        .show(ui, |ui| {
                            for (i, game) in games_snap.iter().enumerate() {
                                if i > 0 && i % cols == 0 {
                                    ui.end_row();
                                }

                                egui::Frame::none()
                                    .fill(Color32::from_rgb(22, 28, 38))
                                    .rounding(Rounding::same(10.0))
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(35, 45, 65)))
                                    .show(ui, |ui| {
                                        ui.set_max_width(card_w);

                                        // Imagen o placeholder
                                        if let Some(tex) = self.textures.get(&game.appid) {
                                            let img = egui::Image::new(tex)
                                                .max_width(card_w)
                                                .max_height(card_h)
                                                .rounding(Rounding {
                                                    nw: 10.0,
                                                    ne: 10.0,
                                                    sw: 0.0,
                                                    se: 0.0,
                                                });
                                            ui.add(img);
                                        } else {
                                            let (rect, _) = ui.allocate_exact_size(
                                                Vec2::new(card_w, card_h),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(
                                                rect,
                                                Rounding { nw: 10.0, ne: 10.0, sw: 0.0, se: 0.0 },
                                                Color32::from_rgb(25, 32, 45),
                                            );
                                            // Nombre como placeholder
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                &game.name,
                                                FontId::proportional(11.0),
                                                Color32::from_rgb(120, 140, 170),
                                            );
                                        }

                                        ui.add_space(6.0);

                                        // Nombre
                                        ui.horizontal(|ui| {
                                            ui.add_space(8.0);
                                            ui.label(
                                                RichText::new(&game.name)
                                                    .font(FontId::proportional(12.5))
                                                    .color(Color32::WHITE)
                                                    .strong(),
                                            );
                                        });

                                        // Horas
                                        ui.horizontal(|ui| {
                                            ui.add_space(8.0);
                                            let hours = game.playtime_hours();
                                            let label = if hours < 1.0 {
                                                format!("{} min jugados", game.playtime_forever)
                                            } else {
                                                format!("{:.0}h jugadas", hours)
                                            };
                                            ui.label(
                                                RichText::new(label)
                                                    .font(FontId::proportional(11.0))
                                                    .color(Color32::from_rgb(120, 140, 170)),
                                            );
                                        });

                                        ui.add_space(8.0);

                                        // Botones
                                        ui.horizontal(|ui| {
                                            ui.add_space(8.0);

                                            let play = ui.add(
                                                egui::Button::new(
                                                    RichText::new("JUGAR")
                                                        .font(FontId::proportional(12.0))
                                                        .color(Color32::WHITE)
                                                        .strong(),
                                                )
                                                .fill(Color32::from_rgb(30, 110, 200))
                                                .rounding(Rounding::same(5.0))
                                                .min_size(Vec2::new(90.0, 26.0)),
                                            );

                                            if play.clicked() {
                                                launch = Some(game.clone());
                                            }

                                            ui.add_space(4.0);

                                            let store = ui.add(
                                                egui::Button::new(
                                                    RichText::new("Tienda")
                                                        .font(FontId::proportional(11.0))
                                                        .color(Color32::from_rgb(160, 180, 210)),
                                                )
                                                .fill(Color32::from_rgb(28, 36, 52))
                                                .rounding(Rounding::same(5.0))
                                                .min_size(Vec2::new(60.0, 26.0)),
                                            );

                                            if store.clicked() {
                                                open_store = Some(game.clone());
                                            }
                                        });

                                        ui.add_space(8.0);
                                    });
                            }
                        });

                    ui.add_space(20.0);
                });
            });

        // Ejecutar acciones fuera del borrow
        if let Some(game) = launch {
            open::that(game.launch_url()).ok();
            self.last_launched = Some(game.name);
            ctx.request_repaint();
        }
        if let Some(game) = open_store {
            open::that(game.store_url()).ok();
        }

        // Repintar si hay imagenes pendientes o cargando
        if !self.pending_images.lock().unwrap().is_empty()
            || *self.loading_games.lock().unwrap()
        {
            ctx.request_repaint();
        }
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

        // Autocargar si entramos directo sin pasar por setup
        {
            let games = self.games.lock().unwrap();
            let loading = *self.loading_games.lock().unwrap();
            if self.screen == Screen::Main && games.is_empty() && !loading {
                drop(games);
                self.start_loading_games();
            }
        }

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
            .with_inner_size([960.0, 660.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Steam Lite",
        options,
        Box::new(|cc| Box::new(SteamLite::new(cc))),
    )
}
