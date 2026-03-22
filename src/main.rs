#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
    steam_path: String,
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

#[derive(Debug, Clone)]
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

// ===================== STEAM LOCAL READER =====================

fn find_steam_path_auto() -> Option<String> {
    let candidates = vec![
        r"C:\Program Files (x86)\Steam",
        r"C:\Program Files\Steam",
        r"D:\Steam",
        r"D:\Program Files (x86)\Steam",
        r"E:\Steam",
        r"C:\Steam",
    ];

    for path in &candidates {
        let p = PathBuf::from(path);
        if p.join("steamapps").exists() {
            return Some(path.to_string());
        }
    }

    // Intentar registro de Windows
    let output = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if line.to_lowercase().contains("steampath") {
            let parts: Vec<&str> = line.split("    ").collect();
            if let Some(last) = parts.last() {
                let path = last.trim().replace("/", "\\");
                if PathBuf::from(&path).join("steamapps").exists() {
                    return Some(path);
                }
            }
        }
    }

    None
}

fn parse_acf_value(content: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\"", key);
    let pos = content.find(&search)?;
    let after = &content[pos + search.len()..];
    let trimmed = after.trim_start();
    if trimmed.starts_with('"') {
        let inner = &trimmed[1..];
        let end = inner.find('"')?;
        Some(inner[..end].to_string())
    } else {
        None
    }
}

fn fetch_games_local(steam_path: &str) -> Result<Vec<Game>, String> {
    let steamapps = PathBuf::from(steam_path).join("steamapps");

    if !steamapps.exists() {
        return Err(format!(
            "No se encontro la carpeta steamapps en:\n{}",
            steamapps.display()
        ));
    }

    let mut games = Vec::new();

    let entries = fs::read_dir(&steamapps)
        .map_err(|e| format!("Error leyendo steamapps: {}", e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if !filename.starts_with("appmanifest_") || !filename.ends_with(".acf") {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let appid: u64 = match parse_acf_value(&content, "appid").and_then(|s| s.parse().ok()) {
            Some(id) => id,
            None => continue,
        };

        let name = match parse_acf_value(&content, "name") {
            Some(n) => n,
            None => continue,
        };

        let playtime = parse_acf_value(&content, "playtime_forever")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0u64);

        games.push(Game {
            appid,
            name,
            playtime_forever: playtime,
        });
    }

    if games.is_empty() {
        return Err(format!(
            "No se encontraron juegos instalados en:\n{}\n\nAsegurate de tener juegos instalados.",
            steamapps.display()
        ));
    }

    games.sort_by(|a, b| b.playtime_forever.cmp(&a.playtime_forever));
    Ok(games)
}

fn fetch_image(url: &str) -> Option<Vec<u8>> {
    let resp = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?
        .get(url)
        .send()
        .ok()?;

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
    input_steam_path: String,
    setup_error: String,
    autodetected: bool,

    // Library
    games: Arc<Mutex<Vec<Game>>>,
    loading_games: Arc<Mutex<bool>>,
    load_error: Arc<Mutex<String>>,
    search: String,

    // Texturas
    textures: HashMap<u64, egui::TextureHandle>,
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

        // Intentar autodetectar Steam
        let autodetected_path = find_steam_path_auto().unwrap_or_default();

        Self {
            screen,
            config,
            input_steam_path: autodetected_path.clone(),
            setup_error: String::new(),
            autodetected: !autodetected_path.is_empty(),
            games: Arc::new(Mutex::new(vec![])),
            loading_games: Arc::new(Mutex::new(false)),
            load_error: Arc::new(Mutex::new(String::new())),
            search: String::new(),
            textures: HashMap::new(),
            pending_images: Arc::new(Mutex::new(vec![])),
            fetching: Arc::new(Mutex::new(std::collections::HashSet::new())),
            last_launched: None,
        }
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
            match fetch_games_local(&cfg.steam_path) {
                Ok(g) => *games.lock().unwrap() = g,
                Err(e) => *error.lock().unwrap() = e,
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
        let fetching_arc = Arc::clone(&self.fetching);

        thread::spawn(move || {
            if let Some(bytes) = fetch_image(&url) {
                pending.lock().unwrap().push((appid, bytes));
            }
            fetching_arc.lock().unwrap().remove(&appid);
        });
    }

    fn validate_and_save(&mut self) {
        let steam_path = self.input_steam_path.trim().to_string();

        if steam_path.is_empty() {
            self.setup_error = "Introduce la ruta de Steam".to_string();
            return;
        }

        let steamapps = PathBuf::from(&steam_path).join("steamapps");
        if !steamapps.exists() {
            self.setup_error = format!(
                "No se encontro steamapps en esa ruta.\nPrueba con: C:\\Program Files (x86)\\Steam"
            );
            return;
        }

        let config = Config { steam_path };
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
                ui.add_space(60.0);
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
                            ui.set_max_width(520.0);

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

                            ui.add_space(20.0);

                            // Autodeteccion
                            if self.autodetected {
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(15, 40, 20))
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(30, 90, 40)))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new("Steam detectado automaticamente")
                                                .font(FontId::proportional(12.0))
                                                .color(Color32::from_rgb(100, 220, 120)),
                                        );
                                    });
                                ui.add_space(12.0);
                            }

                            ui.label(
                                RichText::new("Ruta de Steam")
                                    .font(FontId::proportional(14.0))
                                    .color(Color32::from_rgb(180, 200, 230))
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.input_steam_path)
                                    .desired_width(460.0)
                                    .hint_text(r"C:\Program Files (x86)\Steam"),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new("Ruta donde esta instalado Steam (sin la carpeta steamapps)")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(100, 120, 150)),
                            );

                            ui.add_space(20.0);

                            if !self.setup_error.is_empty() {
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(60, 20, 20))
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(&self.setup_error)
                                                .font(FontId::proportional(13.0))
                                                .color(Color32::from_rgb(255, 120, 120)),
                                        );
                                    });
                                ui.add_space(12.0);
                            }

                            let btn = ui.add_sized(
                                Vec2::new(460.0, 46.0),
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
                            ui.set_max_width(520.0);
                            ui.label(
                                RichText::new("Lee los juegos directamente de tu PC. Sin internet ni API Key.")
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
        // Procesar imagenes
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

                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .desired_width(250.0)
                            .hint_text("Buscar juego..."),
                    );

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
                            RichText::new(format!("{} juegos instalados", count))
                                .font(FontId::proportional(13.0))
                                .color(Color32::from_rgb(120, 140, 170)),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Cerrar sesion").clicked() {
                            delete_config();
                            self.config = None;
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

        // Grid
        let games_snap: Vec<Game> = {
            let games = self.games.lock().unwrap();
            let q = self.search.to_lowercase();
            games
                .iter()
                .filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q))
                .cloned()
                .collect()
        };

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
                            RichText::new("No se encontraron juegos instalados")
                                .font(FontId::proportional(18.0))
                                .color(Color32::from_rgb(100, 120, 150)),
                        );
                    });
                    return;
                }

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(12.0);

                    let card_w = 220.0_f32;
                    let card_h = 103.0_f32;
                    let spacing = 12.0_f32;
                    let available = ui.available_width() - 24.0;
                    let cols = ((available + spacing) / (card_w + spacing))
                        .floor()
                        .max(1.0) as usize;

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

                                        if let Some(tex) = self.textures.get(&game.appid) {
                                            ui.add(
                                                egui::Image::new(tex)
                                                    .max_width(card_w)
                                                    .max_height(card_h)
                                                    .rounding(Rounding {
                                                        nw: 10.0,
                                                        ne: 10.0,
                                                        sw: 0.0,
                                                        se: 0.0,
                                                    }),
                                            );
                                        } else {
                                            let (rect, _) = ui.allocate_exact_size(
                                                Vec2::new(card_w, card_h),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(
                                                rect,
                                                Rounding {
                                                    nw: 10.0,
                                                    ne: 10.0,
                                                    sw: 0.0,
                                                    se: 0.0,
                                                },
                                                Color32::from_rgb(25, 32, 45),
                                            );
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                &game.name,
                                                FontId::proportional(11.0),
                                                Color32::from_rgb(120, 140, 170),
                                            );
                                        }

                                        ui.add_space(6.0);

                                        ui.horizontal(|ui| {
                                            ui.add_space(8.0);
                                            ui.label(
                                                RichText::new(&game.name)
                                                    .font(FontId::proportional(12.5))
                                                    .color(Color32::WHITE)
                                                    .strong(),
                                            );
                                        });

                                        ui.horizontal(|ui| {
                                            ui.add_space(8.0);
                                            let hours = game.playtime_hours();
                                            let label = if game.playtime_forever == 0 {
                                                "Sin jugar".to_string()
                                            } else if hours < 1.0 {
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

                                        ui.horizontal(|ui| {
                                            ui.add_space(8.0);

                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("JUGAR")
                                                            .font(FontId::proportional(12.0))
                                                            .color(Color32::WHITE)
                                                            .strong(),
                                                    )
                                                    .fill(Color32::from_rgb(30, 110, 200))
                                                    .rounding(Rounding::same(5.0))
                                                    .min_size(Vec2::new(90.0, 26.0)),
                                                )
                                                .clicked()
                                            {
                                                launch = Some(game.clone());
                                            }

                                            ui.add_space(4.0);

                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("Tienda")
                                                            .font(FontId::proportional(11.0))
                                                            .color(Color32::from_rgb(160, 180, 210)),
                                                    )
                                                    .fill(Color32::from_rgb(28, 36, 52))
                                                    .rounding(Rounding::same(5.0))
                                                    .min_size(Vec2::new(60.0, 26.0)),
                                                )
                                                .clicked()
                                            {
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

        if let Some(game) = launch {
            open::that(game.launch_url()).ok();
            self.last_launched = Some(game.name);
            ctx.request_repaint();
        }
        if let Some(game) = open_store {
            open::that(game.store_url()).ok();
        }

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
