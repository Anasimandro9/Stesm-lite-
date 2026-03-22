#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use eframe::egui::{Color32, FontId, RichText, Rounding, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

// ===================== CONFIG =====================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    steam_path: String,
    steam_user: Option<String>,
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

// ===================== STRUCTS =====================

#[derive(Debug, Clone)]
struct Game {
    appid: u64,
    name: String,
    playtime_forever: u64,
    installed: bool,
}

impl Game {
    fn playtime_hours(&self) -> f32 {
        self.playtime_forever as f32 / 60.0
    }
    fn image_url(&self) -> String {
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{}/header.jpg", self.appid)
    }
    fn launch_url(&self) -> String {
        format!("steam://run/{}", self.appid)
    }
    fn install_url(&self) -> String {
        format!("steam://install/{}", self.appid)
    }
    fn store_url(&self) -> String {
        format!("https://store.steampowered.com/app/{}", self.appid)
    }
}

#[derive(Debug, Clone)]
struct Friend {
    name: String,
    state: u8,
    game: Option<String>,
    avatar_id: Option<String>,
}

impl Friend {
    fn status_text(&self) -> &str {
        match self.state {
            0 => "Desconectado",
            1 => "Conectado",
            2 => "Ocupado",
            3 => "Ausente",
            _ => "Desconocido",
        }
    }
    fn status_color(&self) -> Color32 {
        match self.state {
            0 => Color32::from_rgb(90, 90, 90),
            1 => Color32::from_rgb(80, 200, 100),
            2 => Color32::from_rgb(200, 80, 80),
            3 => Color32::from_rgb(200, 170, 40),
            _ => Color32::from_rgb(120, 120, 120),
        }
    }
}

// ===================== STEAM LOCAL =====================

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
        if PathBuf::from(path).join("steamapps").exists() {
            return Some(path.to_string());
        }
    }
    let output = Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .output().ok()?;
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
    let steam_pb = PathBuf::from(steam_path);
    let steamapps = steam_pb.join("steamapps");
    if !steamapps.exists() {
        return Err(format!("No se encontro steamapps en:\n{}", steamapps.display()));
    }

    let mut installed: HashMap<u64, Game> = HashMap::new();
    if let Ok(entries) = fs::read_dir(&steamapps) {
        for entry in entries.flatten() {
            let path = entry.path();
            let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if !filename.starts_with("appmanifest_") || !filename.ends_with(".acf") { continue; }
            let content = match fs::read_to_string(&path) { Ok(c) => c, Err(_) => continue };
            let appid: u64 = match parse_acf_value(&content, "appid").and_then(|s| s.parse().ok()) { Some(id) => id, None => continue };
            let name = match parse_acf_value(&content, "name") { Some(n) => n, None => continue };
            let playtime = parse_acf_value(&content, "playtime_forever").and_then(|s| s.parse().ok()).unwrap_or(0u64);
            installed.insert(appid, Game { appid, name, playtime_forever: playtime, installed: true });
        }
    }

    let mut all_games: HashMap<u64, Game> = installed.clone();
    let userdata = steam_pb.join("userdata");
    if let Ok(users) = fs::read_dir(&userdata) {
        for user_entry in users.flatten() {
            let localconfig = user_entry.path().join("config").join("localconfig.vdf");
            if !localconfig.exists() { continue; }
            let content = match fs::read_to_string(&localconfig) { Ok(c) => c, Err(_) => continue };

            let lines: Vec<&str> = content.lines().collect();
            let mut i = 0;
            let mut in_apps = false;
            let mut depth_apps: i32 = 0;
            let mut cur_appid: Option<u64> = None;
            let mut cur_playtime: u64 = 0;

            while i < lines.len() {
                let trimmed = lines[i].trim();
                if !in_apps {
                    if trimmed.to_lowercase().contains("\"apps\"") { in_apps = true; }
                    i += 1; continue;
                }
                if trimmed == "{" { depth_apps += 1; }
                else if trimmed == "}" {
                    if depth_apps == 2 {
                        if let Some(appid) = cur_appid {
                            if !all_games.contains_key(&appid) {
                                all_games.insert(appid, Game {
                                    appid,
                                    name: format!("App {}", appid),
                                    playtime_forever: cur_playtime,
                                    installed: false,
                                });
                            }
                        }
                        cur_appid = None; cur_playtime = 0;
                    }
                    depth_apps -= 1;
                    if depth_apps <= 0 { break; }
                } else if depth_apps == 1 {
                    if let Ok(id) = trimmed.trim_matches('"').parse::<u64>() { cur_appid = Some(id); }
                } else if depth_apps == 2 {
                    let tl = trimmed.to_lowercase();
                    if tl.contains("\"playtime\"") {
                        let val = trimmed.split('"').nth(3).unwrap_or("0");
                        cur_playtime = val.parse().unwrap_or(0);
                    }
                }
                i += 1;
            }
        }
    }

    let installed_ids: HashSet<u64> = installed.keys().cloned().collect();
    let mut games: Vec<Game> = all_games.into_values().collect();
    if games.is_empty() {
        return Err("No se encontraron juegos.".to_string());
    }
    games.sort_by(|a, b| {
        let ai = installed_ids.contains(&a.appid);
        let bi = installed_ids.contains(&b.appid);
        bi.cmp(&ai).then(b.playtime_forever.cmp(&a.playtime_forever))
    });
    Ok(games)
}

fn fetch_friends_local(steam_path: &str) -> Vec<Friend> {
    let mut friends = Vec::new();
    let steam_pb = PathBuf::from(steam_path);
    let userdata = steam_pb.join("userdata");

    if let Ok(users) = fs::read_dir(&userdata) {
        for user_entry in users.flatten() {
            let localconfig = user_entry.path().join("config").join("localconfig.vdf");
            if !localconfig.exists() { continue; }
            let content = match fs::read_to_string(&localconfig) { Ok(c) => c, Err(_) => continue };

            // Buscar sección Friends
            let mut in_friends = false;
            let mut depth: i32 = 0;
            let mut cur_name: Option<String> = None;
            let mut cur_state: u8 = 0;
            let mut cur_game: Option<String> = None;

            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.to_lowercase().contains("\"friends\"") { in_friends = true; continue; }
                if !in_friends { continue; }
                if trimmed == "{" { depth += 1; }
                else if trimmed == "}" {
                    if depth == 2 {
                        if let Some(name) = cur_name.clone() {
                            friends.push(Friend { name, state: cur_state, game: cur_game.clone(), avatar_id: None });
                        }
                        cur_name = None; cur_state = 0; cur_game = None;
                    }
                    depth -= 1;
                    if depth <= 0 { break; }
                } else if depth == 2 {
                    let parts: Vec<&str> = trimmed.splitn(2, '\t').collect();
                    if parts.len() == 2 {
                        let k = parts[0].trim().trim_matches('"').to_lowercase();
                        let v = parts[1].trim().trim_matches('"');
                        match k.as_str() {
                            "name" => cur_name = Some(v.to_string()),
                            "personastate" => cur_state = v.parse().unwrap_or(0),
                            "gamename" => cur_game = Some(v.to_string()),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    friends.sort_by(|a, b| b.state.cmp(&a.state));
    friends
}

fn fetch_image(url: &str) -> Option<Vec<u8>> {
    let resp = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(10))
        .build().ok()?.get(url).send().ok()?;
    if resp.status().is_success() { resp.bytes().ok().map(|b| b.to_vec()) } else { None }
}

// ===================== STEAMCMD =====================

fn find_steamcmd(steam_path: &str) -> Option<PathBuf> {
    let candidates = vec![
        PathBuf::from(steam_path).join("steamcmd.exe"),
        PathBuf::from(steam_path).join("steamapps").join("steamcmd.exe"),
        PathBuf::from(r"C:\steamcmd\steamcmd.exe"),
    ];
    for p in candidates { if p.exists() { return Some(p); } }
    None
}

fn download_game_steamcmd(steam_path: &str, user: &str, appid: u64, status: Arc<Mutex<String>>) {
    let steamcmd = match find_steamcmd(steam_path) {
        Some(p) => p,
        None => {
            // Abrir con steam:// como fallback
            open::that(format!("steam://install/{}", appid)).ok();
            *status.lock().unwrap() = "Abriendo Steam para instalar...".to_string();
            return;
        }
    };

    *status.lock().unwrap() = format!("Descargando app {}...", appid);

    let result = Command::new(&steamcmd)
        .args([
            "+login", user,
            "+app_update", &appid.to_string(), "validate",
            "+quit",
        ])
        .output();

    match result {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains("Success") {
                *status.lock().unwrap() = format!("Descarga completada!");
            } else {
                // SteamCMD no disponible o necesita contraseña - usar steam://
                open::that(format!("steam://install/{}", appid)).ok();
                *status.lock().unwrap() = "Abriendo Steam para instalar...".to_string();
            }
        }
        Err(_) => {
            open::that(format!("steam://install/{}", appid)).ok();
            *status.lock().unwrap() = "Abriendo Steam para instalar...".to_string();
        }
    }
}

// ===================== SCREENS =====================

#[derive(PartialEq, Clone)]
enum Tab { Library, Friends, Settings }

#[derive(PartialEq)]
enum Screen { Setup, Main }

// ===================== APP =====================

struct SteamLite {
    screen: Screen,
    config: Option<Config>,
    input_steam_path: String,
    setup_error: String,
    autodetected: bool,

    games: Arc<Mutex<Vec<Game>>>,
    friends: Arc<Mutex<Vec<Friend>>>,
    loading_games: Arc<Mutex<bool>>,
    load_error: Arc<Mutex<String>>,
    download_status: Arc<Mutex<String>>,

    search: String,
    tab: Tab,
    textures: HashMap<u64, egui::TextureHandle>,
    pending_images: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
    fetching: Arc<Mutex<HashSet<u64>>>,
    last_launched: Option<String>,
}

impl SteamLite {
    fn new(_cc: &eframe::CreationContext) -> Self {
        let config = load_config();
        let screen = if config.is_some() { Screen::Main } else { Screen::Setup };
        let autodetected_path = find_steam_path_auto().unwrap_or_default();

        Self {
            screen, config,
            input_steam_path: autodetected_path.clone(),
            setup_error: String::new(),
            autodetected: !autodetected_path.is_empty(),
            games: Arc::new(Mutex::new(vec![])),
            friends: Arc::new(Mutex::new(vec![])),
            loading_games: Arc::new(Mutex::new(false)),
            load_error: Arc::new(Mutex::new(String::new())),
            download_status: Arc::new(Mutex::new(String::new())),
            search: String::new(),
            tab: Tab::Library,
            textures: HashMap::new(),
            pending_images: Arc::new(Mutex::new(vec![])),
            fetching: Arc::new(Mutex::new(HashSet::new())),
            last_launched: None,
        }
    }

    fn start_loading(&self) {
        let cfg = match &self.config { Some(c) => c.clone(), None => return };
        let games = Arc::clone(&self.games);
        let friends = Arc::clone(&self.friends);
        let loading = Arc::clone(&self.loading_games);
        let error = Arc::clone(&self.load_error);
        *loading.lock().unwrap() = true;
        *error.lock().unwrap() = String::new();
        thread::spawn(move || {
            match fetch_games_local(&cfg.steam_path) {
                Ok(g) => *games.lock().unwrap() = g,
                Err(e) => *error.lock().unwrap() = e,
            }
            let f = fetch_friends_local(&cfg.steam_path);
            *friends.lock().unwrap() = f;
            *loading.lock().unwrap() = false;
        });
    }

    fn request_image(&self, appid: u64, url: String) {
        let mut fetching = self.fetching.lock().unwrap();
        if fetching.contains(&appid) { return; }
        fetching.insert(appid);
        drop(fetching);
        let pending = Arc::clone(&self.pending_images);
        let fa = Arc::clone(&self.fetching);
        thread::spawn(move || {
            if let Some(bytes) = fetch_image(&url) { pending.lock().unwrap().push((appid, bytes)); }
            fa.lock().unwrap().remove(&appid);
        });
    }

    fn validate_and_save(&mut self) {
        let steam_path = self.input_steam_path.trim().to_string();
        if steam_path.is_empty() { self.setup_error = "Introduce la ruta de Steam".to_string(); return; }
        if !PathBuf::from(&steam_path).join("steamapps").exists() {
            self.setup_error = format!("No se encontro steamapps en esa ruta.\nPrueba: C:\\Program Files (x86)\\Steam");
            return;
        }
        let config = Config { steam_path, steam_user: None };
        save_config(&config);
        self.config = Some(config);
        self.setup_error = String::new();
        self.screen = Screen::Main;
        self.start_loading();
    }
}

// ===================== SETUP =====================

impl SteamLite {
    fn show_setup(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(12, 15, 22)))
            .show(ctx, |ui| {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🎮").font(FontId::proportional(64.0)));
                    ui.add_space(8.0);
                    ui.label(RichText::new("STEAM LITE").font(FontId::proportional(38.0)).color(Color32::from_rgb(100, 200, 255)).strong());
                    ui.label(RichText::new("Cliente ligero para PCs con pocos recursos").font(FontId::proportional(14.0)).color(Color32::from_rgb(120, 140, 165)));
                    ui.add_space(36.0);

                    egui::Frame::none()
                        .fill(Color32::from_rgb(20, 26, 38))
                        .rounding(Rounding::same(14.0))
                        .inner_margin(egui::Margin::same(32.0))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(40, 55, 80)))
                        .show(ui, |ui| {
                            ui.set_max_width(520.0);
                            ui.label(RichText::new("Configuracion inicial").font(FontId::proportional(20.0)).color(Color32::WHITE).strong());
                            ui.label(RichText::new("Solo necesitas hacerlo una vez").font(FontId::proportional(13.0)).color(Color32::from_rgb(100, 120, 150)));
                            ui.add_space(20.0);

                            if self.autodetected {
                                egui::Frame::none().fill(Color32::from_rgb(15, 40, 20)).rounding(Rounding::same(6.0))
                                    .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(30, 90, 40)))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("Steam detectado automaticamente").font(FontId::proportional(12.0)).color(Color32::from_rgb(100, 220, 120)));
                                    });
                                ui.add_space(12.0);
                            }

                            ui.label(RichText::new("Ruta de Steam").font(FontId::proportional(14.0)).color(Color32::from_rgb(180, 200, 230)).strong());
                            ui.add_space(4.0);
                            ui.add(egui::TextEdit::singleline(&mut self.input_steam_path).desired_width(460.0).hint_text(r"C:\Program Files (x86)\Steam"));
                            ui.add_space(4.0);
                            ui.label(RichText::new("Carpeta donde esta instalado Steam").font(FontId::proportional(11.0)).color(Color32::from_rgb(100, 120, 150)));
                            ui.add_space(20.0);

                            if !self.setup_error.is_empty() {
                                egui::Frame::none().fill(Color32::from_rgb(60, 20, 20)).rounding(Rounding::same(6.0))
                                    .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(&self.setup_error).font(FontId::proportional(13.0)).color(Color32::from_rgb(255, 120, 120)));
                                    });
                                ui.add_space(12.0);
                            }

                            let btn = ui.add_sized(Vec2::new(460.0, 46.0),
                                egui::Button::new(RichText::new("ENTRAR").font(FontId::proportional(17.0)).color(Color32::WHITE).strong())
                                    .fill(Color32::from_rgb(30, 110, 200)).rounding(Rounding::same(8.0)));
                            if btn.clicked() { self.validate_and_save(); }
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) { self.validate_and_save(); }
                        });

                    ui.add_space(20.0);
                    egui::Frame::none().fill(Color32::from_rgb(15, 30, 20)).rounding(Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(20.0, 10.0))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(30, 80, 40)))
                        .show(ui, |ui| {
                            ui.set_max_width(520.0);
                            ui.label(RichText::new("Lee los juegos directamente de tu PC. Sin API Key.").font(FontId::proportional(12.0)).color(Color32::from_rgb(100, 180, 120)));
                        });
                });
            });
    }
}

// ===================== MAIN =====================

impl SteamLite {
    fn show_main(&mut self, ctx: &egui::Context) {
        // Procesar texturas
        {
            let mut pending = self.pending_images.lock().unwrap();
            for (appid, bytes) in pending.drain(..) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    let img = img.to_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    let pixels: Vec<egui::Color32> = img.pixels().map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
                    let texture = ctx.load_texture(format!("game_{}", appid), egui::ColorImage { size, pixels }, egui::TextureOptions::LINEAR);
                    self.textures.insert(appid, texture);
                }
            }
        }

        // Top bar
        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::none().fill(Color32::from_rgb(10, 13, 20)).inner_margin(egui::Margin::symmetric(16.0, 10.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("STEAM LITE").font(FontId::proportional(20.0)).color(Color32::from_rgb(100, 200, 255)).strong());
                    ui.add_space(20.0);

                    for (label, t) in [("Biblioteca", Tab::Library), ("Amigos", Tab::Friends), ("Ajustes", Tab::Settings)] {
                        let selected = self.tab == t;
                        let color = if selected { Color32::from_rgb(100, 200, 255) } else { Color32::from_rgb(160, 170, 185) };
                        let btn = ui.add(egui::Button::new(RichText::new(label).font(FontId::proportional(14.0)).color(color))
                            .fill(if selected { Color32::from_rgb(20, 40, 65) } else { Color32::TRANSPARENT })
                            .rounding(Rounding::same(6.0)));
                        if btn.clicked() { self.tab = t; }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Cerrar sesion").clicked() {
                            delete_config(); self.config = None; self.screen = Screen::Setup;
                            *self.games.lock().unwrap() = vec![];
                        }
                        let ds = self.download_status.lock().unwrap().clone();
                        if !ds.is_empty() {
                            ui.label(RichText::new(&ds).font(FontId::proportional(12.0)).color(Color32::from_rgb(100, 220, 120)));
                            ui.add_space(10.0);
                        } else if let Some(name) = &self.last_launched {
                            ui.label(RichText::new(format!("Jugando: {}", name)).font(FontId::proportional(12.0)).color(Color32::from_rgb(100, 220, 120)));
                            ui.add_space(10.0);
                        }
                    });
                });
            });

        // Error
        let error = self.load_error.lock().unwrap().clone();
        if !error.is_empty() {
            egui::TopBottomPanel::top("err").frame(egui::Frame::none().fill(Color32::from_rgb(80, 20, 20)).inner_margin(egui::Margin::symmetric(16.0, 8.0))).show(ctx, |ui| {
                ui.label(RichText::new(&error).color(Color32::from_rgb(255, 150, 150)).font(FontId::proportional(13.0)));
            });
        }

        let tab = self.tab.clone();
        match tab {
            Tab::Library => self.show_library(ctx),
            Tab::Friends => self.show_friends(ctx),
            Tab::Settings => self.show_settings(ctx),
        }

        if !self.pending_images.lock().unwrap().is_empty() || *self.loading_games.lock().unwrap() {
            ctx.request_repaint();
        }
    }

    fn show_library(&mut self, ctx: &egui::Context) {
        // Search bar
        egui::TopBottomPanel::top("search").frame(egui::Frame::none().fill(Color32::from_rgb(15, 18, 25)).inner_margin(egui::Margin::symmetric(16.0, 8.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Buscar:").color(Color32::from_rgb(140, 160, 180)));
                ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(280.0).hint_text("Nombre del juego..."));
                let count = self.games.lock().unwrap().len();
                let loading = *self.loading_games.lock().unwrap();
                ui.add_space(10.0);
                if loading {
                    ui.label(RichText::new("Cargando...").color(Color32::from_rgb(200, 180, 50)));
                } else {
                    ui.label(RichText::new(format!("{} juegos", count)).color(Color32::from_rgb(120, 140, 170)));
                }
            });
        });

        let games_snap: Vec<Game> = {
            let games = self.games.lock().unwrap();
            let q = self.search.to_lowercase();
            games.iter().filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q)).cloned().collect()
        };

        for game in games_snap.iter().take(30) {
            if !self.textures.contains_key(&game.appid) {
                self.request_image(game.appid, game.image_url());
            }
        }

        let cfg = self.config.clone();
        let mut launch: Option<Game> = None;
        let mut install: Option<Game> = None;
        let mut open_store: Option<Game> = None;

        egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::from_rgb(13, 16, 23))).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(12.0);
                let card_w = 220.0_f32;
                let card_h = 103.0_f32;
                let spacing = 12.0_f32;
                let available = ui.available_width() - 24.0;
                let cols = ((available + spacing) / (card_w + spacing)).floor().max(1.0) as usize;

                egui::Grid::new("lib").num_columns(cols).spacing(Vec2::splat(spacing)).min_col_width(card_w).show(ui, |ui| {
                    for (i, game) in games_snap.iter().enumerate() {
                        if i > 0 && i % cols == 0 { ui.end_row(); }

                        egui::Frame::none()
                            .fill(Color32::from_rgb(22, 28, 38))
                            .rounding(Rounding::same(10.0))
                            .stroke(Stroke::new(1.0, if game.installed { Color32::from_rgb(30, 60, 100) } else { Color32::from_rgb(35, 45, 65) }))
                            .show(ui, |ui| {
                                ui.set_max_width(card_w);

                                if let Some(tex) = self.textures.get(&game.appid) {
                                    ui.add(egui::Image::new(tex).max_width(card_w).max_height(card_h).rounding(Rounding { nw: 10.0, ne: 10.0, sw: 0.0, se: 0.0 }));
                                } else {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::new(card_w, card_h), egui::Sense::hover());
                                    ui.painter().rect_filled(rect, Rounding { nw: 10.0, ne: 10.0, sw: 0.0, se: 0.0 }, Color32::from_rgb(25, 32, 45));
                                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &game.name, FontId::proportional(10.0), Color32::from_rgb(120, 140, 170));
                                }

                                ui.add_space(5.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(8.0);
                                    ui.label(RichText::new(&game.name).font(FontId::proportional(12.0)).color(Color32::WHITE).strong());
                                });
                                ui.horizontal(|ui| {
                                    ui.add_space(8.0);
                                    if game.installed {
                                        ui.label(RichText::new("● Instalado").font(FontId::proportional(10.0)).color(Color32::from_rgb(80, 200, 100)));
                                    } else {
                                        let hours = game.playtime_hours();
                                        let label = if game.playtime_forever == 0 { "Sin jugar".to_string() } else if hours < 1.0 { format!("{} min", game.playtime_forever) } else { format!("{:.0}h", hours) };
                                        ui.label(RichText::new(label).font(FontId::proportional(10.0)).color(Color32::from_rgb(120, 140, 170)));
                                    }
                                });

                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(8.0);
                                    if game.installed {
                                        if ui.add(egui::Button::new(RichText::new("JUGAR").font(FontId::proportional(12.0)).color(Color32::WHITE).strong())
                                            .fill(Color32::from_rgb(30, 110, 200)).rounding(Rounding::same(5.0)).min_size(Vec2::new(85.0, 26.0))).clicked() {
                                            launch = Some(game.clone());
                                        }
                                    } else {
                                        if ui.add(egui::Button::new(RichText::new("INSTALAR").font(FontId::proportional(11.0)).color(Color32::WHITE).strong())
                                            .fill(Color32::from_rgb(30, 140, 60)).rounding(Rounding::same(5.0)).min_size(Vec2::new(85.0, 26.0))).clicked() {
                                            install = Some(game.clone());
                                        }
                                    }
                                    ui.add_space(4.0);
                                    if ui.add(egui::Button::new(RichText::new("Tienda").font(FontId::proportional(11.0)).color(Color32::from_rgb(160, 180, 210)))
                                        .fill(Color32::from_rgb(28, 36, 52)).rounding(Rounding::same(5.0)).min_size(Vec2::new(55.0, 26.0))).clicked() {
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
        if let Some(game) = install {
            if let Some(cfg) = cfg {
                let steam_path = cfg.steam_path.clone();
                let user = cfg.steam_user.clone().unwrap_or_else(|| "anonymous".to_string());
                let status = Arc::clone(&self.download_status);
                let appid = game.appid;
                thread::spawn(move || { download_game_steamcmd(&steam_path, &user, appid, status); });
            }
        }
        if let Some(game) = open_store { open::that(game.store_url()).ok(); }
    }

    fn show_friends(&mut self, ctx: &egui::Context) {
        let friends = self.friends.lock().unwrap().clone();

        egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::from_rgb(13, 16, 23))).show(ctx, |ui| {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("Amigos").font(FontId::proportional(22.0)).color(Color32::WHITE).strong());
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} amigos", friends.len())).font(FontId::proportional(13.0)).color(Color32::from_rgb(120, 140, 170)));
            });
            ui.add_space(12.0);

            if friends.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("No se encontraron amigos en los archivos locales de Steam.\nAbre Steam una vez para sincronizar los datos.").font(FontId::proportional(14.0)).color(Color32::from_rgb(100, 120, 150)));
                });
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                for friend in &friends {
                    ui.add_space(4.0);
                    egui::Frame::none().fill(Color32::from_rgb(20, 26, 36)).rounding(Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(32, 42, 58)))
                        .show(ui, |ui| {
                            ui.set_max_width(500.0);
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), egui::Sense::hover());
                                ui.painter().circle_filled(rect.center(), 5.0, friend.status_color());
                                ui.add_space(8.0);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&friend.name).font(FontId::proportional(14.0)).color(Color32::WHITE).strong());
                                    if let Some(game) = &friend.game {
                                        ui.label(RichText::new(format!("Jugando: {}", game)).font(FontId::proportional(11.0)).color(Color32::from_rgb(100, 200, 120)));
                                    } else {
                                        ui.label(RichText::new(friend.status_text()).font(FontId::proportional(11.0)).color(Color32::from_rgb(120, 140, 170)));
                                    }
                                });
                            });
                        });
                }
                ui.add_space(20.0);
            });
        });
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::from_rgb(13, 16, 23))).show(ctx, |ui| {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("Ajustes").font(FontId::proportional(22.0)).color(Color32::WHITE).strong());
            });
            ui.add_space(16.0);

            ui.horizontal(|ui| {
                ui.add_space(20.0);
                egui::Frame::none().fill(Color32::from_rgb(20, 26, 38)).rounding(Rounding::same(12.0))
                    .inner_margin(egui::Margin::same(24.0))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(35, 48, 70)))
                    .show(ui, |ui| {
                        ui.set_max_width(480.0);

                        if let Some(cfg) = &self.config {
                            ui.label(RichText::new("Ruta de Steam").font(FontId::proportional(13.0)).color(Color32::from_rgb(140, 160, 180)));
                            ui.label(RichText::new(&cfg.steam_path).font(FontId::proportional(13.0)).color(Color32::WHITE));
                            ui.add_space(16.0);
                        }

                        if ui.add(egui::Button::new(RichText::new("Recargar biblioteca").color(Color32::WHITE))
                            .fill(Color32::from_rgb(30, 90, 160)).rounding(Rounding::same(6.0)).min_size(Vec2::new(200.0, 34.0))).clicked() {
                            self.start_loading();
                        }

                        ui.add_space(8.0);

                        if ui.add(egui::Button::new(RichText::new("Abrir Steam para descargar").color(Color32::WHITE))
                            .fill(Color32::from_rgb(25, 70, 130)).rounding(Rounding::same(6.0)).min_size(Vec2::new(200.0, 34.0))).clicked() {
                            Command::new("cmd").args(["/C", "start", "", r"C:\Program Files (x86)\Steam\steam.exe"]).spawn().ok();
                        }

                        ui.add_space(16.0);
                        ui.separator();
                        ui.add_space(12.0);

                        if ui.add(egui::Button::new(RichText::new("Resetear configuracion").color(Color32::from_rgb(255, 120, 120)))
                            .fill(Color32::from_rgb(55, 20, 20)).rounding(Rounding::same(6.0)).min_size(Vec2::new(200.0, 34.0))).clicked() {
                            delete_config();
                            self.config = None;
                            self.screen = Screen::Setup;
                            *self.games.lock().unwrap() = vec![];
                        }
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

        {
            let games = self.games.lock().unwrap();
            let loading = *self.loading_games.lock().unwrap();
            if self.screen == Screen::Main && games.is_empty() && !loading {
                drop(games);
                self.start_loading();
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
    eframe::run_native("Steam Lite", options, Box::new(|cc| Box::new(SteamLite::new(cc))))
}
