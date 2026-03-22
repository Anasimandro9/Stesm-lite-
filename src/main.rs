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

// ===================== COLORES STEAM =====================
const BG_DARK:      Color32 = Color32::from_rgb(23, 26, 33);
const BG_MID:       Color32 = Color32::from_rgb(27, 40, 56);
const BG_CARD:      Color32 = Color32::from_rgb(22, 32, 45);
const BG_HOVER:     Color32 = Color32::from_rgb(30, 45, 65);
const BLUE_STEAM:   Color32 = Color32::from_rgb(102, 192, 244);
const GREEN_STEAM:  Color32 = Color32::from_rgb(75, 200, 90);
const BTN_BLUE:     Color32 = Color32::from_rgb(59, 133, 177);
const BTN_GREEN:    Color32 = Color32::from_rgb(54, 150, 64);
const TEXT_MAIN:    Color32 = Color32::from_rgb(198, 210, 220);
const TEXT_DIM:     Color32 = Color32::from_rgb(100, 130, 155);
const BORDER:       Color32 = Color32::from_rgb(40, 60, 80);

// ===================== CONFIG =====================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    steam_path: String,
    steam_user: Option<String>,
    api_key: Option<String>,
}

fn config_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.pop();
    p.push("steamlite_config.json");
    p
}

fn load_config() -> Option<Config> {
    serde_json::from_str(&fs::read_to_string(config_path()).ok()?).ok()
}

fn save_config(config: &Config) {
    if let Ok(json) = serde_json::to_string_pretty(config) { fs::write(config_path(), json).ok(); }
}

fn delete_config() { fs::remove_file(config_path()).ok(); }

// ===================== STRUCTS =====================

#[derive(Debug, Clone)]
struct Game {
    appid: u64,
    name: String,
    playtime_forever: u64,
    installed: bool,
}

impl Game {
    fn playtime_str(&self) -> String {
        let h = self.playtime_forever as f32 / 60.0;
        if self.playtime_forever == 0 { "Sin jugar".into() }
        else if h < 1.0 { format!("{} min", self.playtime_forever) }
        else { format!("{:.1} horas", h) }
    }
    fn image_url(&self) -> String { format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{}/header.jpg", self.appid) }
    fn launch_url(&self) -> String { format!("steam://run/{}", self.appid) }
    fn store_url(&self) -> String { format!("https://store.steampowered.com/app/{}", self.appid) }
}

#[derive(Debug, Clone)]
struct Friend {
    name: String,
    state: u8,
    game: Option<String>,
}

impl Friend {
    fn status_text(&self) -> &str { match self.state { 1 => "Conectado", 2 => "Ocupado", 3 => "Ausente", _ => "Desconectado" } }
    fn state_color(&self) -> Color32 { match self.state { 1 => GREEN_STEAM, 2 => Color32::from_rgb(200,80,80), 3 => Color32::from_rgb(200,160,40), _ => TEXT_DIM } }
}

// ===================== STEAM LOCAL =====================

fn find_steam_path_auto() -> Option<String> {
    let paths = [r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam", r"D:\Steam", r"E:\Steam"];
    for p in &paths { if PathBuf::from(p).join("steamapps").exists() { return Some(p.to_string()); } }
    let out = Command::new("reg").args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"]).output().ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.to_lowercase().contains("steampath") {
            if let Some(last) = line.split("    ").last() {
                let p = last.trim().replace("/", "\\");
                if PathBuf::from(&p).join("steamapps").exists() { return Some(p); }
            }
        }
    }
    None
}

fn parse_acf(content: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\"", key);
    let pos = content.find(&search)?;
    let after = content[pos + search.len()..].trim_start();
    if after.starts_with('"') { let inner = &after[1..]; Some(inner[..inner.find('"')?].to_string()) } else { None }
}

fn load_games(steam_path: &str) -> Result<Vec<Game>, String> {
    let pb = PathBuf::from(steam_path);
    let apps = pb.join("steamapps");
    if !apps.exists() { return Err(format!("No se encontro steamapps en {}", apps.display())); }

    let mut installed: HashMap<u64, Game> = HashMap::new();
    if let Ok(entries) = fs::read_dir(&apps) {
        for e in entries.flatten() {
            let path = e.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") { continue; }
            let content = match fs::read_to_string(&path) { Ok(c) => c, _ => continue };
            let appid: u64 = match parse_acf(&content, "appid").and_then(|s| s.parse().ok()) { Some(id) => id, _ => continue };
            let gname = match parse_acf(&content, "name") { Some(n) => n, _ => continue };
            let pt = parse_acf(&content, "playtime_forever").and_then(|s| s.parse().ok()).unwrap_or(0u64);
            installed.insert(appid, Game { appid, name: gname, playtime_forever: pt, installed: true });
        }
    }

    let mut all: HashMap<u64, Game> = installed.clone();
    let userdata = pb.join("userdata");
    if let Ok(users) = fs::read_dir(&userdata) {
        for ue in users.flatten() {
            let lc = ue.path().join("config").join("localconfig.vdf");
            if !lc.exists() { continue; }
            let content = match fs::read_to_string(&lc) { Ok(c) => c, _ => continue };
            let lines: Vec<&str> = content.lines().collect();
            let mut i = 0;
            let mut in_apps = false;
            let mut depth: i32 = 0;
            let mut cur_id: Option<u64> = None;
            let mut cur_pt: u64 = 0;
            while i < lines.len() {
                let t = lines[i].trim();
                if !in_apps { if t.to_lowercase().contains("\"apps\"") { in_apps = true; } i += 1; continue; }
                if t == "{" { depth += 1; }
                else if t == "}" {
                    if depth == 2 {
                        if let Some(id) = cur_id {
                            if !all.contains_key(&id) {
                                all.insert(id, Game { appid: id, name: format!("App {}", id), playtime_forever: cur_pt, installed: false });
                            }
                        }
                        cur_id = None; cur_pt = 0;
                    }
                    depth -= 1; if depth <= 0 { break; }
                } else if depth == 1 {
                    if let Ok(id) = t.trim_matches('"').parse::<u64>() { cur_id = Some(id); }
                } else if depth == 2 && t.to_lowercase().contains("\"playtime\"") {
                    cur_pt = t.split('"').nth(3).unwrap_or("0").parse().unwrap_or(0);
                }
                i += 1;
            }
        }
    }

    let inst_ids: HashSet<u64> = installed.keys().cloned().collect();
    let mut games: Vec<Game> = all.into_values().collect();
    if games.is_empty() { return Err("No se encontraron juegos.".into()); }
    games.sort_by(|a, b| inst_ids.contains(&b.appid).cmp(&inst_ids.contains(&a.appid)).then(b.playtime_forever.cmp(&a.playtime_forever)));
    Ok(games)
}

fn load_friends(steam_path: &str) -> Vec<Friend> {
    let mut friends = Vec::new();
    if let Ok(users) = fs::read_dir(PathBuf::from(steam_path).join("userdata")) {
        for ue in users.flatten() {
            let lc = ue.path().join("config").join("localconfig.vdf");
            if !lc.exists() { continue; }
            let content = match fs::read_to_string(&lc) { Ok(c) => c, _ => continue };
            let mut in_f = false; let mut depth: i32 = 0;
            let mut name: Option<String> = None; let mut state: u8 = 0; let mut game: Option<String> = None;
            for line in content.lines() {
                let t = line.trim();
                if t.to_lowercase().contains("\"friends\"") { in_f = true; continue; }
                if !in_f { continue; }
                if t == "{" { depth += 1; }
                else if t == "}" {
                    if depth == 2 { if let Some(n) = name.clone() { friends.push(Friend { name: n, state, game: game.clone() }); } name = None; state = 0; game = None; }
                    depth -= 1; if depth <= 0 { break; }
                } else if depth == 2 {
                    let parts: Vec<&str> = t.splitn(2, '\t').collect();
                    if parts.len() == 2 {
                        let k = parts[0].trim().trim_matches('"').to_lowercase();
                        let v = parts[1].trim().trim_matches('"');
                        match k.as_str() { "name" => name = Some(v.into()), "personastate" => state = v.parse().unwrap_or(0), "gamename" => game = Some(v.into()), _ => {} }
                    }
                }
            }
        }
    }
    friends.sort_by(|a, b| b.state.cmp(&a.state));
    friends
}

fn fetch_game_names_api(api_key: &str, steam_id: &str) -> HashMap<u64, String> {
    let url = format!("https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?key={}&steamid={}&include_appinfo=true", api_key, steam_id);
    let resp = match reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(15)).build().ok().and_then(|c| c.get(&url).send().ok()) { Some(r) => r, None => return HashMap::new() };
    #[derive(serde::Deserialize)] struct R { response: Ri }
    #[derive(serde::Deserialize)] struct Ri { games: Option<Vec<Ge>> }
    #[derive(serde::Deserialize)] struct Ge { appid: u64, name: String }
    match resp.json::<R>() { Ok(data) => data.response.games.unwrap_or_default().into_iter().map(|g| (g.appid, g.name)).collect(), _ => HashMap::new() }
}

fn fetch_image(url: &str) -> Option<Vec<u8>> {
    let r = reqwest::blocking::Client::builder().user_agent("Mozilla/5.0").timeout(std::time::Duration::from_secs(10)).build().ok()?.get(url).send().ok()?;
    if r.status().is_success() { r.bytes().ok().map(|b| b.to_vec()) } else { None }
}

fn find_steamcmd(steam_path: &str) -> Option<PathBuf> {
    for p in [PathBuf::from(steam_path).join("steamcmd.exe"), PathBuf::from(r"C:\steamcmd\steamcmd.exe")] { if p.exists() { return Some(p); } }
    None
}

fn download_game(steam_path: &str, user: &str, pass: &str, guard: &str, appid: u64, status: Arc<Mutex<String>>) {
    *status.lock().unwrap() = "Conectando con Steam...".into();
    let cmd = match find_steamcmd(steam_path) { Some(p) => p, None => { open::that(format!("steam://install/{}", appid)).ok(); *status.lock().unwrap() = "Abriendo Steam para instalar...".into(); return; } };
    let install_dir = PathBuf::from(steam_path).join("steamapps").join("common");
    let mut args = vec!["+login".into(), user.into(), pass.into()];
    if !guard.is_empty() { args.push(guard.into()); }
    args.extend(["+force_install_dir".into(), install_dir.to_string_lossy().into_owned(), "+app_update".into(), appid.to_string(), "validate".into(), "+quit".into()]);
    match Command::new(&cmd).args(&args).output() {
        Ok(out) => {
            let t = String::from_utf8_lossy(&out.stdout);
            if t.contains("Success") { *status.lock().unwrap() = "Descarga completada!".into(); }
            else if t.contains("guard") { *status.lock().unwrap() = "Introduce el codigo Steam Guard en Ajustes.".into(); }
            else if t.contains("Invalid") { *status.lock().unwrap() = "Contrasena incorrecta.".into(); }
            else { open::that(format!("steam://install/{}", appid)).ok(); *status.lock().unwrap() = "Abriendo Steam...".into(); }
        }
        Err(_) => { open::that(format!("steam://install/{}", appid)).ok(); *status.lock().unwrap() = "Abriendo Steam...".into(); }
    }
}

// ===================== APP STATE =====================

#[derive(PartialEq, Clone)] enum Tab { Library, Friends, Settings }
#[derive(PartialEq)] enum Screen { Setup, Main }

struct App {
    screen: Screen,
    config: Option<Config>,
    input_path: String,
    setup_error: String,
    autodetected: bool,

    games: Arc<Mutex<Vec<Game>>>,
    friends: Arc<Mutex<Vec<Friend>>>,
    loading: Arc<Mutex<bool>>,
    load_error: Arc<Mutex<String>>,
    dl_status: Arc<Mutex<String>>,

    search: String,
    tab: Tab,
    textures: HashMap<u64, egui::TextureHandle>,
    pending_imgs: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
    fetching: Arc<Mutex<HashSet<u64>>>,
    last_played: Option<String>,

    steamcmd_user: String,
    steamcmd_pass: String,
    steamcmd_guard: String,
    show_login_popup: bool,
    pending_appid: Option<u64>,

    // API key para nombres reales
    input_api_key: String,
    input_steam_id: String,
}

impl App {
    fn new(_cc: &eframe::CreationContext) -> Self {
        let config = load_config();
        let screen = if config.is_some() { Screen::Main } else { Screen::Setup };
        let auto = find_steam_path_auto().unwrap_or_default();
        let user = config.as_ref().and_then(|c| c.steam_user.clone()).unwrap_or_default();
        let api_key = config.as_ref().and_then(|c| c.api_key.clone()).unwrap_or_default();
        Self {
            screen, config, input_path: auto.clone(), setup_error: String::new(), autodetected: !auto.is_empty(),
            games: Arc::new(Mutex::new(vec![])), friends: Arc::new(Mutex::new(vec![])),
            loading: Arc::new(Mutex::new(false)), load_error: Arc::new(Mutex::new(String::new())), dl_status: Arc::new(Mutex::new(String::new())),
            search: String::new(), tab: Tab::Library, textures: HashMap::new(),
            pending_imgs: Arc::new(Mutex::new(vec![])), fetching: Arc::new(Mutex::new(HashSet::new())), last_played: None,
            steamcmd_user: user, steamcmd_pass: String::new(), steamcmd_guard: String::new(), show_login_popup: false, pending_appid: None,
            input_api_key: api_key, input_steam_id: String::new(),
        }
    }

    fn reload(&self) {
        let cfg = match &self.config { Some(c) => c.clone(), None => return };
        let games = Arc::clone(&self.games);
        let friends = Arc::clone(&self.friends);
        let loading = Arc::clone(&self.loading);
        let error = Arc::clone(&self.load_error);
        let api_key = cfg.api_key.clone();
        *loading.lock().unwrap() = true;
        *error.lock().unwrap() = String::new();
        thread::spawn(move || {
            match load_games(&cfg.steam_path) {
                Ok(mut g) => {
                    // Si hay API key, actualizar nombres reales
                    if let Some(key) = api_key {
                        if !key.is_empty() {
                            // Extraer steam_id del primer juego para la llamada
                            // Usamos la key para obtener nombres reales
                            let names = fetch_game_names_api(&key, "");
                            if !names.is_empty() {
                                for game in &mut g {
                                    if let Some(n) = names.get(&game.appid) { game.name = n.clone(); }
                                }
                            }
                        }
                    }
                    *games.lock().unwrap() = g;
                }
                Err(e) => *error.lock().unwrap() = e,
            }
            *friends.lock().unwrap() = load_friends(&cfg.steam_path);
            *loading.lock().unwrap() = false;
        });
    }

    fn req_image(&self, appid: u64, url: String) {
        let mut f = self.fetching.lock().unwrap();
        if f.contains(&appid) { return; } f.insert(appid); drop(f);
        let p = Arc::clone(&self.pending_imgs); let fa = Arc::clone(&self.fetching);
        thread::spawn(move || { if let Some(b) = fetch_image(&url) { p.lock().unwrap().push((appid, b)); } fa.lock().unwrap().remove(&appid); });
    }

    fn do_setup(&mut self) {
        let path = self.input_path.trim().to_string();
        if path.is_empty() { self.setup_error = "Introduce la ruta de Steam".into(); return; }
        if !PathBuf::from(&path).join("steamapps").exists() { self.setup_error = "No se encontro steamapps en esa ruta".into(); return; }
        let config = Config { steam_path: path, steam_user: None, api_key: None };
        save_config(&config);
        self.config = Some(config);
        self.setup_error = String::new();
        self.screen = Screen::Main;
        self.reload();
    }

    fn do_download(&mut self, appid: u64) {
        if self.steamcmd_user.is_empty() || self.steamcmd_pass.is_empty() { self.pending_appid = Some(appid); self.show_login_popup = true; return; }
        let path = self.config.as_ref().map(|c| c.steam_path.clone()).unwrap_or_default();
        let user = self.steamcmd_user.clone(); let pass = self.steamcmd_pass.clone(); let guard = self.steamcmd_guard.clone();
        let status = Arc::clone(&self.dl_status);
        if let Some(cfg) = &mut self.config { cfg.steam_user = Some(user.clone()); save_config(cfg); }
        thread::spawn(move || { download_game(&path, &user, &pass, &guard, appid, status); });
    }
}

// ===================== SETUP SCREEN =====================

impl App {
    fn show_setup(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG_DARK)).show(ctx, |ui| {
            ui.add_space(50.0);
            ui.vertical_centered(|ui| {
                // Logo
                ui.label(RichText::new("STEAM LITE").font(FontId::proportional(44.0)).color(BLUE_STEAM).strong());
                ui.add_space(4.0);
                ui.label(RichText::new("Cliente ligero — bajo consumo de RAM").font(FontId::proportional(14.0)).color(TEXT_DIM));
                ui.add_space(40.0);

                egui::Frame::none().fill(BG_MID).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::same(28.0)).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
                    ui.set_max_width(480.0);
                    ui.label(RichText::new("Configuracion inicial").font(FontId::proportional(18.0)).color(TEXT_MAIN).strong());
                    ui.add_space(20.0);

                    if self.autodetected {
                        egui::Frame::none().fill(Color32::from_rgb(20, 50, 25)).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(12.0, 6.0))
                            .show(ui, |ui| { ui.label(RichText::new("✓ Steam detectado automaticamente").font(FontId::proportional(12.0)).color(GREEN_STEAM)); });
                        ui.add_space(10.0);
                    }

                    ui.label(RichText::new("Ruta de Steam").font(FontId::proportional(13.0)).color(TEXT_DIM));
                    ui.add_space(3.0);
                    ui.add(egui::TextEdit::singleline(&mut self.input_path).desired_width(424.0).hint_text(r"C:\Program Files (x86)\Steam").text_color(TEXT_MAIN));

                    if !self.setup_error.is_empty() {
                        ui.add_space(10.0);
                        ui.label(RichText::new(&self.setup_error).color(Color32::from_rgb(255, 100, 100)).font(FontId::proportional(12.0)));
                    }

                    ui.add_space(20.0);
                    let btn = ui.add_sized(Vec2::new(424.0, 40.0), egui::Button::new(RichText::new("ENTRAR").font(FontId::proportional(15.0)).color(Color32::WHITE).strong()).fill(BTN_BLUE).rounding(Rounding::same(4.0)));
                    if btn.clicked() { self.do_setup(); }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) { self.do_setup(); }
                });

                ui.add_space(16.0);
                ui.label(RichText::new("Tus datos se guardan solo en tu PC.").font(FontId::proportional(12.0)).color(TEXT_DIM));
            });
        });
    }
}

// ===================== MAIN SCREEN =====================

impl App {
    fn show_main(&mut self, ctx: &egui::Context) {
        // Procesar texturas
        {
            let mut pending = self.pending_imgs.lock().unwrap();
            for (appid, bytes) in pending.drain(..) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    let img = img.to_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    let pixels = img.pixels().map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
                    let tex = ctx.load_texture(format!("g{}", appid), egui::ColorImage { size, pixels }, egui::TextureOptions::LINEAR);
                    self.textures.insert(appid, tex);
                }
            }
        }

        // SIDEBAR izquierda estilo Steam
        egui::SidePanel::left("sidebar").exact_width(200.0)
            .frame(egui::Frame::none().fill(Color32::from_rgb(18, 22, 30)).inner_margin(egui::Margin::symmetric(0.0, 0.0)))
            .show(ctx, |ui| {
                ui.add_space(0.0);

                // Header
                egui::Frame::none().fill(Color32::from_rgb(10, 14, 20)).inner_margin(egui::Margin::symmetric(16.0, 14.0)).show(ui, |ui| {
                    ui.label(RichText::new("STEAM LITE").font(FontId::proportional(18.0)).color(BLUE_STEAM).strong());
                });

                ui.add_space(8.0);

                // Nav items
                let tabs = [("🎮  BIBLIOTECA", Tab::Library), ("👥  AMIGOS", Tab::Friends), ("⚙   AJUSTES", Tab::Settings)];
                for (label, t) in &tabs {
                    let selected = self.tab == *t;
                    let bg = if selected { Color32::from_rgb(30, 55, 80) } else { Color32::TRANSPARENT };
                    let fg = if selected { BLUE_STEAM } else { TEXT_MAIN };
                    let btn = egui::Frame::none().fill(bg).inner_margin(egui::Margin::symmetric(16.0, 8.0))
                        .show(ui, |ui| {
                            ui.set_min_width(200.0);
                            ui.label(RichText::new(*label).font(FontId::proportional(13.0)).color(fg));
                        });
                    if btn.response.interact(egui::Sense::click()).clicked() { self.tab = t.clone(); }
                }

                // Status bottom
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);

                let ds = self.dl_status.lock().unwrap().clone();
                if !ds.is_empty() {
                    egui::Frame::none().inner_margin(egui::Margin::symmetric(12.0, 4.0)).show(ui, |ui| {
                        ui.label(RichText::new(&ds).font(FontId::proportional(11.0)).color(GREEN_STEAM));
                    });
                } else if let Some(name) = &self.last_played {
                    egui::Frame::none().inner_margin(egui::Margin::symmetric(12.0, 4.0)).show(ui, |ui| {
                        ui.label(RichText::new(format!("▶ {}", name)).font(FontId::proportional(11.0)).color(GREEN_STEAM));
                    });
                }
            });

        // Error banner
        let error = self.load_error.lock().unwrap().clone();
        if !error.is_empty() {
            egui::TopBottomPanel::top("err").frame(egui::Frame::none().fill(Color32::from_rgb(100, 20, 20)).inner_margin(egui::Margin::symmetric(16.0, 8.0))).show(ctx, |ui| {
                ui.label(RichText::new(&error).color(Color32::from_rgb(255, 180, 180)).font(FontId::proportional(13.0)));
            });
        }

        // Login popup SteamCMD
        if self.show_login_popup { self.show_login_popup(ctx); }

        let tab = self.tab.clone();
        match tab {
            Tab::Library => self.show_library(ctx),
            Tab::Friends => self.show_friends(ctx),
            Tab::Settings => self.show_settings(ctx),
        }

        if !self.pending_imgs.lock().unwrap().is_empty() || *self.loading.lock().unwrap() { ctx.request_repaint(); }
    }

    fn show_library(&mut self, ctx: &egui::Context) {
        // Barra superior
        egui::TopBottomPanel::top("topbar").frame(egui::Frame::none().fill(Color32::from_rgb(20, 25, 35)).inner_margin(egui::Margin::symmetric(16.0, 8.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(260.0).hint_text("Buscar juego...").text_color(TEXT_MAIN).frame(true));
                ui.add_space(12.0);
                let cnt = self.games.lock().unwrap().len();
                let loading = *self.loading.lock().unwrap();
                if loading { ui.label(RichText::new("Cargando...").color(Color32::from_rgb(200, 180, 50)).font(FontId::proportional(13.0))); }
                else { ui.label(RichText::new(format!("{} juegos", cnt)).color(TEXT_DIM).font(FontId::proportional(13.0))); }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Recargar").font(FontId::proportional(12.0)).color(TEXT_MAIN)).fill(Color32::from_rgb(35, 45, 60)).rounding(Rounding::same(4.0))).clicked() { self.reload(); }
                });
            });
        });

        let games_snap: Vec<Game> = {
            let gs = self.games.lock().unwrap();
            let q = self.search.to_lowercase();
            gs.iter().filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q)).cloned().collect()
        };

        for g in games_snap.iter().take(30) { if !self.textures.contains_key(&g.appid) { self.req_image(g.appid, g.image_url()); } }

        let mut launch: Option<Game> = None;
        let mut install_id: Option<u64> = None;
        let mut open_url: Option<String> = None;

        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG_DARK)).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(12.0);
                let cw = 220.0_f32;
                let ch = 103.0_f32;
                let sp = 10.0_f32;
                let cols = ((ui.available_width() - 20.0 + sp) / (cw + sp)).floor().max(1.0) as usize;

                egui::Grid::new("grid").num_columns(cols).spacing(Vec2::splat(sp)).min_col_width(cw).show(ui, |ui| {
                    for (i, g) in games_snap.iter().enumerate() {
                        if i > 0 && i % cols == 0 { ui.end_row(); }

                        let border_color = if g.installed { Color32::from_rgb(50, 90, 130) } else { BORDER };
                        egui::Frame::none().fill(BG_CARD).rounding(Rounding::same(6.0)).stroke(Stroke::new(1.0, border_color)).show(ui, |ui| {
                            ui.set_max_width(cw);

                            // Imagen
                            if let Some(tex) = self.textures.get(&g.appid) {
                                ui.add(egui::Image::new(tex).max_width(cw).max_height(ch).rounding(Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 }));
                            } else {
                                let (rect, _) = ui.allocate_exact_size(Vec2::new(cw, ch), egui::Sense::hover());
                                ui.painter().rect_filled(rect, Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 }, Color32::from_rgb(20, 28, 40));
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &g.name, FontId::proportional(10.0), TEXT_DIM);
                            }

                            // Info
                            egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 5.0)).show(ui, |ui| {
                                ui.label(RichText::new(&g.name).font(FontId::proportional(11.5)).color(TEXT_MAIN).strong());
                                ui.horizontal(|ui| {
                                    if g.installed {
                                        ui.label(RichText::new("● Instalado").font(FontId::proportional(10.0)).color(GREEN_STEAM));
                                    } else {
                                        ui.label(RichText::new(g.playtime_str()).font(FontId::proportional(10.0)).color(TEXT_DIM));
                                    }
                                });
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    if g.installed {
                                        if ui.add(egui::Button::new(RichText::new("JUGAR").font(FontId::proportional(11.0)).color(Color32::WHITE).strong()).fill(BTN_BLUE).rounding(Rounding::same(3.0)).min_size(Vec2::new(80.0, 24.0))).clicked() { launch = Some(g.clone()); }
                                    } else {
                                        if ui.add(egui::Button::new(RichText::new("INSTALAR").font(FontId::proportional(10.0)).color(Color32::WHITE).strong()).fill(BTN_GREEN).rounding(Rounding::same(3.0)).min_size(Vec2::new(80.0, 24.0))).clicked() { install_id = Some(g.appid); }
                                    }
                                    ui.add_space(3.0);
                                    if ui.add(egui::Button::new(RichText::new("Tienda").font(FontId::proportional(10.0)).color(TEXT_DIM)).fill(Color32::from_rgb(25, 35, 50)).rounding(Rounding::same(3.0)).min_size(Vec2::new(48.0, 24.0))).clicked() { open_url = Some(g.store_url()); }
                                });
                                ui.add_space(3.0);
                            });
                        });
                    }
                });
                ui.add_space(20.0);
            });
        });

        if let Some(g) = launch { open::that(g.launch_url()).ok(); self.last_played = Some(g.name); ctx.request_repaint(); }
        if let Some(id) = install_id { self.do_download(id); }
        if let Some(url) = open_url { open::that(url).ok(); }
    }

    fn show_friends(&mut self, ctx: &egui::Context) {
        let friends = self.friends.lock().unwrap().clone();
        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG_DARK)).show(ctx, |ui| {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("Amigos").font(FontId::proportional(20.0)).color(TEXT_MAIN).strong());
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{}", friends.len())).font(FontId::proportional(13.0)).color(TEXT_DIM));
            });
            ui.add_space(12.0);

            if friends.is_empty() {
                ui.centered_and_justified(|ui| { ui.label(RichText::new("Abre Steam una vez para sincronizar amigos.").font(FontId::proportional(14.0)).color(TEXT_DIM)); });
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                for f in &friends {
                    egui::Frame::none().fill(BG_CARD).rounding(Rounding::same(6.0)).inner_margin(egui::Margin::symmetric(14.0, 10.0)).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
                        ui.set_max_width(480.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::new(8.0, 8.0), egui::Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, f.state_color());
                            ui.add_space(8.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&f.name).font(FontId::proportional(13.0)).color(TEXT_MAIN).strong());
                                if let Some(g) = &f.game { ui.label(RichText::new(format!("Jugando: {}", g)).font(FontId::proportional(11.0)).color(GREEN_STEAM)); }
                                else { ui.label(RichText::new(f.status_text()).font(FontId::proportional(11.0)).color(TEXT_DIM)); }
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
                ui.add_space(20.0);
            });
        });
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG_DARK)).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(16.0);
                ui.horizontal(|ui| { ui.add_space(16.0); ui.label(RichText::new("Ajustes").font(FontId::proportional(20.0)).color(TEXT_MAIN).strong()); });
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_max_width(500.0);

                        // Seccion API Key
                        egui::Frame::none().fill(BG_MID).rounding(Rounding::same(6.0)).inner_margin(egui::Margin::same(18.0)).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
                            ui.label(RichText::new("API Key (para nombres reales)").font(FontId::proportional(14.0)).color(TEXT_MAIN).strong());
                            ui.add_space(6.0);
                            ui.label(RichText::new("Consigue tu key gratis en steamwebapi.com sin comprar nada").font(FontId::proportional(11.0)).color(TEXT_DIM));
                            ui.add_space(8.0);
                            ui.label(RichText::new("steamwebapi.com API Key").font(FontId::proportional(12.0)).color(TEXT_DIM));
                            ui.add(egui::TextEdit::singleline(&mut self.input_api_key).desired_width(380.0).hint_text("Tu API Key de steamwebapi.com").password(true).text_color(TEXT_MAIN));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Tu Steam ID (64-bit)").font(FontId::proportional(12.0)).color(TEXT_DIM));
                            ui.add(egui::TextEdit::singleline(&mut self.input_steam_id).desired_width(380.0).hint_text("76561199528188579").text_color(TEXT_MAIN));
                            ui.add_space(10.0);
                            if ui.add(egui::Button::new(RichText::new("Guardar y recargar").color(Color32::WHITE)).fill(BTN_BLUE).rounding(Rounding::same(4.0)).min_size(Vec2::new(160.0, 30.0))).clicked() {
                                if let Some(cfg) = &mut self.config {
                                    cfg.api_key = Some(self.input_api_key.clone());
                                    save_config(cfg);
                                }
                                self.reload();
                            }
                        });

                        ui.add_space(12.0);

                        // Seccion SteamCMD
                        egui::Frame::none().fill(BG_MID).rounding(Rounding::same(6.0)).inner_margin(egui::Margin::same(18.0)).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
                            ui.label(RichText::new("SteamCMD — Descargar juegos").font(FontId::proportional(14.0)).color(TEXT_MAIN).strong());
                            ui.add_space(8.0);
                            ui.label(RichText::new("Usuario").font(FontId::proportional(12.0)).color(TEXT_DIM));
                            ui.add(egui::TextEdit::singleline(&mut self.steamcmd_user).desired_width(300.0).hint_text("usuario_steam").text_color(TEXT_MAIN));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Contrasena").font(FontId::proportional(12.0)).color(TEXT_DIM));
                            ui.add(egui::TextEdit::singleline(&mut self.steamcmd_pass).desired_width(300.0).password(true).hint_text("••••••••").text_color(TEXT_MAIN));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Steam Guard (si lo pide)").font(FontId::proportional(12.0)).color(TEXT_DIM));
                            ui.add(egui::TextEdit::singleline(&mut self.steamcmd_guard).desired_width(200.0).hint_text("Codigo de 5 digitos").text_color(TEXT_MAIN));
                        });

                        ui.add_space(12.0);

                        // Acciones
                        egui::Frame::none().fill(BG_MID).rounding(Rounding::same(6.0)).inner_margin(egui::Margin::same(18.0)).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
                            ui.label(RichText::new("Acciones").font(FontId::proportional(14.0)).color(TEXT_MAIN).strong());
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new(RichText::new("Recargar biblioteca").color(Color32::WHITE)).fill(BTN_BLUE).rounding(Rounding::same(4.0)).min_size(Vec2::new(150.0, 30.0))).clicked() { self.reload(); }
                                ui.add_space(8.0);
                                if ui.add(egui::Button::new(RichText::new("Abrir Steam").color(Color32::WHITE)).fill(Color32::from_rgb(40, 55, 75)).rounding(Rounding::same(4.0)).min_size(Vec2::new(110.0, 30.0))).clicked() {
                                    Command::new("cmd").args(["/C", "start", "", r"C:\Program Files (x86)\Steam\steam.exe"]).spawn().ok();
                                }
                            });
                            ui.add_space(12.0);
                            if let Some(cfg) = &self.config {
                                ui.label(RichText::new(format!("Ruta: {}", cfg.steam_path)).font(FontId::proportional(11.0)).color(TEXT_DIM));
                            }
                            ui.add_space(10.0);
                            if ui.add(egui::Button::new(RichText::new("Resetear configuracion").color(Color32::from_rgb(255, 120, 120))).fill(Color32::from_rgb(60, 20, 20)).rounding(Rounding::same(4.0)).min_size(Vec2::new(180.0, 30.0))).clicked() {
                                delete_config(); self.config = None; self.screen = Screen::Setup; *self.games.lock().unwrap() = vec![];
                            }
                        });
                    });
                });
            });
        });
    }

    fn show_login_popup(&mut self, ctx: &egui::Context) {
        let mut open = self.show_login_popup;
        egui::Window::new("Iniciar sesion para descargar").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).open(&mut open).show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.add_space(6.0);
            ui.label(RichText::new("SteamCMD descargara el juego en segundo plano.").font(FontId::proportional(13.0)).color(TEXT_MAIN));
            ui.add_space(12.0);
            ui.label(RichText::new("Usuario").color(TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut self.steamcmd_user).desired_width(320.0).text_color(TEXT_MAIN));
            ui.add_space(8.0);
            ui.label(RichText::new("Contrasena").color(TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut self.steamcmd_pass).desired_width(320.0).password(true).text_color(TEXT_MAIN));
            ui.add_space(8.0);
            ui.label(RichText::new("Steam Guard (si lo pide)").color(TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut self.steamcmd_guard).desired_width(200.0).hint_text("Dejalo vacio").text_color(TEXT_MAIN));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("DESCARGAR").color(Color32::WHITE).strong()).fill(BTN_GREEN).rounding(Rounding::same(4.0)).min_size(Vec2::new(120.0, 32.0))).clicked() {
                    if let Some(appid) = self.pending_appid { self.show_login_popup = false; self.do_download(appid); self.pending_appid = None; }
                }
                ui.add_space(8.0);
                if ui.add(egui::Button::new(RichText::new("Cancelar").color(TEXT_DIM)).fill(Color32::from_rgb(35, 45, 60)).rounding(Rounding::same(4.0)).min_size(Vec2::new(90.0, 32.0))).clicked() {
                    self.show_login_popup = false;
                    if let Some(appid) = self.pending_appid { open::that(format!("steam://install/{}", appid)).ok(); self.pending_appid = None; }
                }
            });
            ui.add_space(6.0);
        });
        if !open { self.show_login_popup = false; }
    }
}

// ===================== APP LOOP =====================

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut style = (*ctx.style()).clone();
        style.visuals.dark_mode = true;
        style.visuals.panel_fill = BG_DARK;
        style.visuals.window_fill = BG_MID;
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(35, 48, 65);
        style.visuals.widgets.hovered.bg_fill = BG_HOVER;
        style.visuals.widgets.inactive.rounding = Rounding::same(4.0);
        style.visuals.selection.bg_fill = BTN_BLUE;
        ctx.set_style(style);

        {
            let gs = self.games.lock().unwrap();
            let ld = *self.loading.lock().unwrap();
            if self.screen == Screen::Main && gs.is_empty() && !ld { drop(gs); self.reload(); }
        }

        match self.screen {
            Screen::Setup => self.show_setup(ctx),
            Screen::Main => self.show_main(ctx),
        }
    }
}

fn main() -> eframe::Result<()> {
    eframe::run_native("Steam Lite", eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Steam Lite").with_inner_size([1100.0, 700.0]).with_min_inner_size([700.0, 500.0]),
        ..Default::default()
    }, Box::new(|cc| Box::new(App::new(cc))))
}
