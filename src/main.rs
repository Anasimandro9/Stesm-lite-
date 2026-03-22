#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use eframe::egui::{Color32, FontId, RichText, Rounding, Stroke, Vec2, Pos2, Rect};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

// ===================== PALETA =====================
const C_BG:         Color32 = Color32::from_rgb(16, 18, 24);
const C_SIDEBAR:    Color32 = Color32::from_rgb(11, 13, 18);
const C_PANEL:      Color32 = Color32::from_rgb(20, 24, 32);
const C_CARD:       Color32 = Color32::from_rgb(24, 29, 40);
const C_TOPBAR:     Color32 = Color32::from_rgb(13, 15, 21);
const C_BORDER:     Color32 = Color32::from_rgb(38, 48, 65);
const C_BORDER_LIT: Color32 = Color32::from_rgb(55, 75, 105);
const C_ACCENT:     Color32 = Color32::from_rgb(102, 192, 244);
const C_GREEN:      Color32 = Color32::from_rgb(74, 197, 90);
const C_TEXT:       Color32 = Color32::from_rgb(195, 208, 220);
const C_TEXT_DIM:   Color32 = Color32::from_rgb(110, 130, 155);
const C_TEXT_FAINT: Color32 = Color32::from_rgb(65, 80, 100);
const C_BTN:        Color32 = Color32::from_rgb(55, 125, 170);
const C_BTN_GREEN:  Color32 = Color32::from_rgb(50, 145, 60);
const C_RED:        Color32 = Color32::from_rgb(200, 70, 70);

// Dimensiones fijas de cards — TODAS IGUALES
const CARD_W: f32 = 200.0;
const CARD_IMG_H: f32 = 94.0; // 460x215 ratio ~ 0.467
const CARD_INFO_H: f32 = 70.0;
const CARD_H: f32 = CARD_IMG_H + CARD_INFO_H;

// ===================== CONFIG =====================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    steam_path: String,
    steam_user: Option<String>,
}

fn config_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.pop(); p.push("steamlite_config.json"); p
}

fn names_cache_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.pop(); p.push("steamlite_names.json"); p
}

fn load_config() -> Option<Config> {
    serde_json::from_str(&fs::read_to_string(config_path()).ok()?).ok()
}

fn save_config(c: &Config) {
    if let Ok(j) = serde_json::to_string_pretty(c) { fs::write(config_path(), j).ok(); }
}

fn delete_config() {
    fs::remove_file(config_path()).ok();
    fs::remove_file(names_cache_path()).ok();
}

fn load_names_cache() -> HashMap<u64, String> {
    fs::read_to_string(names_cache_path()).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_names_cache(names: &HashMap<u64, String>) {
    if let Ok(j) = serde_json::to_string(names) { fs::write(names_cache_path(), j).ok(); }
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
    fn playtime_str(&self) -> String {
        let h = self.playtime_forever as f32 / 60.0;
        if self.playtime_forever == 0 { "Sin jugar".into() }
        else if h < 1.0 { format!("{} min", self.playtime_forever) }
        else { format!("{:.1}h jugadas", h) }
    }
    fn img_url(&self) -> String {
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{}/header.jpg", self.appid)
    }
    fn launch_url(&self) -> String { format!("steam://run/{}", self.appid) }
    fn store_url(&self) -> String { format!("https://store.steampowered.com/app/{}", self.appid) }
    fn has_real_name(&self) -> bool { !self.name.starts_with("App ") }
}

#[derive(Debug, Clone)]
struct Friend { name: String, state: u8, game: Option<String> }

impl Friend {
    fn status_label(&self) -> &str {
        match self.state { 1 => "En línea", 2 => "Ocupado", 3 => "Ausente", _ => "Desconectado" }
    }
    fn state_color(&self) -> Color32 {
        match self.state { 1 => C_GREEN, 2 => Color32::from_rgb(200,80,80), 3 => Color32::from_rgb(200,160,40), _ => C_TEXT_FAINT }
    }
}

// ===================== STEAM LOCAL =====================

fn find_steam() -> Option<String> {
    for p in &[r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam", r"D:\Steam", r"E:\Steam"] {
        if PathBuf::from(p).join("steamapps").exists() { return Some(p.to_string()); }
    }
    let out = Command::new("reg").args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"]).output().ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.to_lowercase().contains("steampath") {
            if let Some(v) = line.split("    ").last() {
                let p = v.trim().replace("/", "\\");
                if PathBuf::from(&p).join("steamapps").exists() { return Some(p); }
            }
        }
    }
    None
}

fn acf_val(s: &str, k: &str) -> Option<String> {
    let search = format!("\"{}\"", k);
    let pos = s.find(&search)?;
    let after = s[pos + search.len()..].trim_start();
    if after.starts_with('"') { let inner = &after[1..]; Some(inner[..inner.find('"')?].to_string()) } else { None }
}

fn load_games(path: &str, names_cache: &HashMap<u64, String>) -> Result<Vec<Game>, String> {
    let pb = PathBuf::from(path);
    let apps = pb.join("steamapps");
    if !apps.exists() { return Err(format!("No se encontró steamapps en {}", path)); }

    let mut installed: HashMap<u64, Game> = HashMap::new();
    if let Ok(entries) = fs::read_dir(&apps) {
        for e in entries.flatten() {
            let p = e.path();
            let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            if !n.starts_with("appmanifest_") || !n.ends_with(".acf") { continue; }
            let c = match fs::read_to_string(&p) { Ok(c) => c, _ => continue };
            let id: u64 = match acf_val(&c, "appid").and_then(|s| s.parse().ok()) { Some(i) => i, _ => continue };
            let name = acf_val(&c, "name").unwrap_or_else(|| names_cache.get(&id).cloned().unwrap_or_else(|| format!("App {}", id)));
            let pt = acf_val(&c, "playtime_forever").and_then(|s| s.parse().ok()).unwrap_or(0u64);
            installed.insert(id, Game { appid: id, name, playtime_forever: pt, installed: true });
        }
    }

    let mut all: HashMap<u64, Game> = installed.clone();
    if let Ok(users) = fs::read_dir(pb.join("userdata")) {
        for ue in users.flatten() {
            let lc = ue.path().join("config").join("localconfig.vdf");
            if !lc.exists() { continue; }
            let content = match fs::read_to_string(&lc) { Ok(c) => c, _ => continue };
            let lines: Vec<&str> = content.lines().collect();
            let mut i = 0; let mut in_apps = false; let mut depth: i32 = 0;
            let mut cur_id: Option<u64> = None; let mut cur_pt: u64 = 0;
            while i < lines.len() {
                let t = lines[i].trim();
                if !in_apps { if t.to_lowercase().contains("\"apps\"") { in_apps = true; } i += 1; continue; }
                if t == "{" { depth += 1; }
                else if t == "}" {
                    if depth == 2 {
                        if let Some(id) = cur_id {
                            if !all.contains_key(&id) {
                                let name = names_cache.get(&id).cloned().unwrap_or_else(|| format!("App {}", id));
                                all.insert(id, Game { appid: id, name, playtime_forever: cur_pt, installed: false });
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

    let inst: HashSet<u64> = installed.keys().cloned().collect();
    let mut games: Vec<Game> = all.into_values().collect();
    if games.is_empty() { return Err("No se encontraron juegos.".into()); }
    games.sort_by(|a, b| inst.contains(&b.appid).cmp(&inst.contains(&a.appid)).then(b.playtime_forever.cmp(&a.playtime_forever)));
    Ok(games)
}

fn load_friends(path: &str) -> Vec<Friend> {
    let mut friends = Vec::new();
    if let Ok(users) = fs::read_dir(PathBuf::from(path).join("userdata")) {
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

// Obtiene nombre real desde la API pública de Steam (sin key)
fn fetch_app_name(appid: u64) -> Option<String> {
    let url = format!("https://store.steampowered.com/api/appdetails?appids={}&filters=basic", appid);
    let resp = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(8))
        .build().ok()?.get(&url).send().ok()?;
    let json: serde_json::Value = resp.json().ok()?;
    json[appid.to_string()]["data"]["name"].as_str().map(|s| s.to_string())
}

fn fetch_img(url: &str) -> Option<Vec<u8>> {
    let r = reqwest::blocking::Client::builder().user_agent("Mozilla/5.0").timeout(std::time::Duration::from_secs(10)).build().ok()?.get(url).send().ok()?;
    if r.status().is_success() { r.bytes().ok().map(|b| b.to_vec()) } else { None }
}

fn find_steamcmd(steam_path: &str) -> Option<PathBuf> {
    for p in [PathBuf::from(steam_path).join("steamcmd.exe"), PathBuf::from(r"C:\steamcmd\steamcmd.exe")] {
        if p.exists() { return Some(p); }
    }
    None
}

fn download_game(steam_path: &str, user: &str, pass: &str, guard: &str, appid: u64, status: Arc<Mutex<String>>) {
    *status.lock().unwrap() = "Conectando con Steam...".into();
    let cmd = match find_steamcmd(steam_path) {
        Some(p) => p,
        None => { open::that(format!("steam://install/{}", appid)).ok(); *status.lock().unwrap() = "Abriendo Steam para instalar...".into(); return; }
    };
    let dir = PathBuf::from(steam_path).join("steamapps").join("common");
    let mut args = vec!["+login".into(), user.into(), pass.into()];
    if !guard.is_empty() { args.push(guard.into()); }
    args.extend(["+force_install_dir".into(), dir.to_string_lossy().into_owned(), "+app_update".into(), appid.to_string(), "validate".into(), "+quit".into()]);
    match Command::new(&cmd).args(&args).output() {
        Ok(out) => {
            let t = String::from_utf8_lossy(&out.stdout);
            if t.contains("Success") { *status.lock().unwrap() = "¡Descarga completada!".into(); }
            else if t.contains("guard") { *status.lock().unwrap() = "Introduce el código Steam Guard en Ajustes.".into(); }
            else if t.contains("Invalid") { *status.lock().unwrap() = "Contraseña incorrecta.".into(); }
            else { open::that(format!("steam://install/{}", appid)).ok(); *status.lock().unwrap() = "Abriendo Steam...".into(); }
        }
        Err(_) => { open::that(format!("steam://install/{}", appid)).ok(); *status.lock().unwrap() = "Abriendo Steam...".into(); }
    }
}

// ===================== APP STATE =====================

#[derive(PartialEq, Clone)] enum Tab { Library, Friends, Settings }
#[derive(PartialEq)] enum Screen { Setup, Main }

struct SteamLite {
    screen: Screen,
    config: Option<Config>,
    input_path: String,
    setup_error: String,
    autodetected: bool,

    games: Arc<Mutex<Vec<Game>>>,
    friends: Arc<Mutex<Vec<Friend>>>,
    names_cache: Arc<Mutex<HashMap<u64, String>>>,
    loading: Arc<Mutex<bool>>,
    resolving_names: Arc<Mutex<bool>>,
    load_error: Arc<Mutex<String>>,
    dl_status: Arc<Mutex<String>>,

    search: String,
    tab: Tab,
    show_installed_only: bool,

    textures: HashMap<u64, egui::TextureHandle>,
    pending_imgs: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
    fetching: Arc<Mutex<HashSet<u64>>>,
    last_played: Option<String>,

    steamcmd_user: String,
    steamcmd_pass: String,
    steamcmd_guard: String,
    show_login: bool,
    pending_appid: Option<u64>,
}

impl SteamLite {
    fn new(_cc: &eframe::CreationContext) -> Self {
        let config = load_config();
        let screen = if config.is_some() { Screen::Main } else { Screen::Setup };
        let auto = find_steam().unwrap_or_default();
        let user = config.as_ref().and_then(|c| c.steam_user.clone()).unwrap_or_default();
        let names_cache = Arc::new(Mutex::new(load_names_cache()));
        Self {
            screen, config, input_path: auto.clone(), setup_error: String::new(), autodetected: !auto.is_empty(),
            games: Arc::new(Mutex::new(vec![])), friends: Arc::new(Mutex::new(vec![])),
            names_cache,
            loading: Arc::new(Mutex::new(false)), resolving_names: Arc::new(Mutex::new(false)),
            load_error: Arc::new(Mutex::new(String::new())), dl_status: Arc::new(Mutex::new(String::new())),
            search: String::new(), tab: Tab::Library, show_installed_only: false,
            textures: HashMap::new(), pending_imgs: Arc::new(Mutex::new(vec![])), fetching: Arc::new(Mutex::new(HashSet::new())), last_played: None,
            steamcmd_user: user, steamcmd_pass: String::new(), steamcmd_guard: String::new(), show_login: false, pending_appid: None,
        }
    }

    fn reload(&self) {
        let cfg = match &self.config { Some(c) => c.clone(), None => return };
        let (games, friends, loading, error, names) = (
            Arc::clone(&self.games), Arc::clone(&self.friends),
            Arc::clone(&self.loading), Arc::clone(&self.load_error),
            Arc::clone(&self.names_cache),
        );
        *loading.lock().unwrap() = true; *error.lock().unwrap() = String::new();
        thread::spawn(move || {
            let cache = names.lock().unwrap().clone();
            match load_games(&cfg.steam_path, &cache) { Ok(g) => *games.lock().unwrap() = g, Err(e) => *error.lock().unwrap() = e }
            *friends.lock().unwrap() = load_friends(&cfg.steam_path);
            *loading.lock().unwrap() = false;
        });
    }

    fn resolve_names(&self) {
        let games_arc = Arc::clone(&self.games);
        let names_arc = Arc::clone(&self.names_cache);
        let resolving = Arc::clone(&self.resolving_names);

        if *resolving.lock().unwrap() { return; }
        *resolving.lock().unwrap() = true;

        thread::spawn(move || {
            // Obtener IDs sin nombre real
            let ids_to_resolve: Vec<u64> = {
                let gs = games_arc.lock().unwrap();
                let cache = names_arc.lock().unwrap();
                gs.iter().filter(|g| !g.has_real_name() && !cache.contains_key(&g.appid)).map(|g| g.appid).take(15).collect()
            };

            let mut updated = false;
            for appid in ids_to_resolve {
                if let Some(name) = fetch_app_name(appid) {
                    let mut cache = names_arc.lock().unwrap();
                    cache.insert(appid, name.clone());
                    drop(cache);
                    // Actualizar nombre en games
                    let mut gs = games_arc.lock().unwrap();
                    if let Some(g) = gs.iter_mut().find(|g| g.appid == appid) { g.name = name; }
                    updated = true;
                }
                thread::sleep(std::time::Duration::from_millis(300)); // respetar rate limit
            }

            if updated {
                let cache = names_arc.lock().unwrap().clone();
                save_names_cache(&cache);
            }

            *resolving.lock().unwrap() = false;
        });
    }

    fn req_img(&self, appid: u64, url: String) {
        let mut f = self.fetching.lock().unwrap();
        if f.contains(&appid) { return; } f.insert(appid); drop(f);
        let (p, fa) = (Arc::clone(&self.pending_imgs), Arc::clone(&self.fetching));
        thread::spawn(move || { if let Some(b) = fetch_img(&url) { p.lock().unwrap().push((appid, b)); } fa.lock().unwrap().remove(&appid); });
    }

    fn do_setup(&mut self) {
        let path = self.input_path.trim().to_string();
        if path.is_empty() { self.setup_error = "Introduce la ruta de Steam".into(); return; }
        if !PathBuf::from(&path).join("steamapps").exists() { self.setup_error = "No se encontró steamapps en esa ruta".into(); return; }
        let cfg = Config { steam_path: path, steam_user: None };
        save_config(&cfg); self.config = Some(cfg); self.setup_error = String::new(); self.screen = Screen::Main; self.reload();
    }

    fn do_download(&mut self, appid: u64) {
        if self.steamcmd_user.is_empty() || self.steamcmd_pass.is_empty() { self.pending_appid = Some(appid); self.show_login = true; return; }
        let path = self.config.as_ref().map(|c| c.steam_path.clone()).unwrap_or_default();
        let (user, pass, guard) = (self.steamcmd_user.clone(), self.steamcmd_pass.clone(), self.steamcmd_guard.clone());
        let status = Arc::clone(&self.dl_status);
        if let Some(cfg) = &mut self.config { cfg.steam_user = Some(user.clone()); save_config(cfg); }
        thread::spawn(move || { download_game(&path, &user, &pass, &guard, appid, status); });
    }
}

// ===================== SETUP =====================

impl SteamLite {
    fn show_setup(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            let rect = ui.max_rect();
            // Línea de acento superior
            ui.painter().rect_filled(Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + 2.0)), Rounding::ZERO, C_ACCENT);

            ui.add_space(90.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("STEAM LITE").font(FontId::proportional(42.0)).color(C_ACCENT).strong());
                ui.add_space(6.0);
                ui.label(RichText::new("Cliente refinado · Bajo consumo de RAM").font(FontId::proportional(13.0)).color(C_TEXT_DIM));
                ui.add_space(44.0);

                egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::same(30.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
                    ui.set_max_width(440.0);
                    ui.label(RichText::new("Configuración inicial").font(FontId::proportional(16.0)).color(C_TEXT).strong());
                    ui.add_space(4.0);
                    ui.label(RichText::new("Solo necesitas hacerlo una vez.").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                    ui.add_space(20.0);

                    if self.autodetected {
                        egui::Frame::none().fill(Color32::from_rgb(18, 45, 22)).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(12.0, 7.0)).stroke(Stroke::new(1.0, Color32::from_rgb(40, 100, 50))).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("✓").color(C_GREEN).font(FontId::proportional(12.0)));
                                ui.add_space(4.0);
                                ui.label(RichText::new("Steam detectado automáticamente").font(FontId::proportional(12.0)).color(C_GREEN));
                            });
                        });
                        ui.add_space(14.0);
                    }

                    ui.label(RichText::new("Ruta de instalación de Steam").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                    ui.add_space(4.0);
                    egui::Frame::none().fill(Color32::from_rgb(13, 16, 22)).rounding(Rounding::same(5.0)).stroke(Stroke::new(1.0, C_BORDER_LIT)).inner_margin(egui::Margin::symmetric(10.0, 7.0)).show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.input_path).desired_width(380.0).hint_text(r"C:\Program Files (x86)\Steam").frame(false).text_color(C_TEXT));
                    });

                    if !self.setup_error.is_empty() {
                        ui.add_space(10.0);
                        egui::Frame::none().fill(Color32::from_rgb(45, 15, 15)).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(12.0, 7.0)).show(ui, |ui| {
                            ui.label(RichText::new(&self.setup_error).color(C_RED).font(FontId::proportional(12.0)));
                        });
                    }

                    ui.add_space(20.0);
                    let btn = ui.add_sized(Vec2::new(400.0, 38.0), egui::Button::new(RichText::new("Entrar →").font(FontId::proportional(14.0)).color(Color32::WHITE).strong()).fill(C_BTN).rounding(Rounding::same(5.0)));
                    if btn.clicked() { self.do_setup(); }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) { self.do_setup(); }
                });

                ui.add_space(18.0);
                ui.label(RichText::new("🔒  Tus datos permanecen en tu PC. Sin telemetría.").font(FontId::proportional(11.0)).color(C_TEXT_FAINT));
            });
        });
    }
}

// ===================== MAIN =====================

impl SteamLite {
    fn show_main(&mut self, ctx: &egui::Context) {
        // Procesar texturas pendientes
        {
            let mut p = self.pending_imgs.lock().unwrap();
            for (id, bytes) in p.drain(..) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    let img = img.to_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    let pixels = img.pixels().map(|px| egui::Color32::from_rgba_unmultiplied(px[0], px[1], px[2], px[3])).collect();
                    let tex = ctx.load_texture(format!("g{}", id), egui::ColorImage { size, pixels }, egui::TextureOptions::LINEAR);
                    self.textures.insert(id, tex);
                }
            }
        }

        // Resolver nombres en background si hay "App XXXXX"
        {
            let has_unnamed = self.games.lock().unwrap().iter().any(|g| !g.has_real_name());
            let resolving = *self.resolving_names.lock().unwrap();
            if has_unnamed && !resolving { self.resolve_names(); }
        }

        // SIDEBAR
        egui::SidePanel::left("sidebar").exact_width(210.0)
            .frame(egui::Frame::none().fill(C_SIDEBAR))
            .show(ctx, |ui| {
                ui.set_min_height(ui.available_height());

                // Header
                egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(18.0, 16.0)).show(ui, |ui| {
                    ui.set_min_width(210.0);
                    ui.label(RichText::new("STEAM LITE").font(FontId::proportional(16.0)).color(C_ACCENT).strong());
                    ui.add_space(3.0);
                    let cnt = self.games.lock().unwrap().len();
                    let loading = *self.loading.lock().unwrap();
                    let resolving = *self.resolving_names.lock().unwrap();
                    if loading {
                        ui.label(RichText::new("Cargando biblioteca...").font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                    } else if resolving {
                        ui.label(RichText::new(format!("{} juegos · Obteniendo nombres...", cnt)).font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                    } else {
                        ui.label(RichText::new(format!("{} juegos en biblioteca", cnt)).font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                    }
                });

                ui.add_space(10.0);

                // Nav items
                for (icon, label, t) in [("🎮", "Biblioteca", Tab::Library), ("👥", "Amigos", Tab::Friends), ("⚙", "Ajustes", Tab::Settings)] {
                    let selected = self.tab == t;
                    let resp = egui::Frame::none()
                        .fill(if selected { Color32::from_rgb(22, 38, 58) } else { Color32::TRANSPARENT })
                        .inner_margin(egui::Margin::symmetric(18.0, 10.0))
                        .show(ui, |ui| {
                            ui.set_min_width(210.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon).font(FontId::proportional(13.0)).color(if selected { C_ACCENT } else { C_TEXT_DIM }));
                                ui.add_space(8.0);
                                ui.label(RichText::new(label).font(FontId::proportional(13.0)).color(if selected { C_TEXT } else { C_TEXT_DIM }));
                            });
                        });
                    // Barra indicadora lateral
                    if selected {
                        let r = resp.response.rect;
                        ui.painter().rect_filled(Rect::from_min_max(r.min, Pos2::new(r.min.x + 3.0, r.max.y)), Rounding::ZERO, C_ACCENT);
                    }
                    if resp.response.interact(egui::Sense::click()).clicked() { self.tab = t; }
                }

                ui.add_space(14.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(12.0);

                // Estado actual
                let ds = self.dl_status.lock().unwrap().clone();
                egui::Frame::none().inner_margin(egui::Margin::symmetric(16.0, 0.0)).show(ui, |ui| {
                    if !ds.is_empty() {
                        ui.label(RichText::new(&ds).font(FontId::proportional(11.0)).color(C_GREEN));
                    } else if let Some(name) = &self.last_played.clone() {
                        ui.label(RichText::new("EN JUEGO").font(FontId::proportional(9.5)).color(C_TEXT_FAINT).strong());
                        ui.add_space(2.0);
                        ui.label(RichText::new(name).font(FontId::proportional(11.5)).color(C_GREEN));
                    }
                });

                // Cerrar sesión abajo
                let avail = ui.available_height();
                if avail > 36.0 { ui.add_space(avail - 36.0); }
                egui::Frame::none().inner_margin(egui::Margin::symmetric(16.0, 6.0)).show(ui, |ui| {
                    if ui.add(egui::Button::new(RichText::new("Cerrar sesión").font(FontId::proportional(11.0)).color(C_TEXT_FAINT)).fill(Color32::TRANSPARENT).rounding(Rounding::same(4.0))).clicked() {
                        delete_config(); self.config = None; self.screen = Screen::Setup; *self.games.lock().unwrap() = vec![];
                    }
                });
            });

        // Error banner
        let error = self.load_error.lock().unwrap().clone();
        if !error.is_empty() {
            egui::TopBottomPanel::top("err").frame(egui::Frame::none().fill(Color32::from_rgb(80, 18, 18)).inner_margin(egui::Margin::symmetric(16.0, 8.0))).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").color(C_RED));
                    ui.add_space(6.0);
                    ui.label(RichText::new(&error).color(Color32::from_rgb(255, 180, 180)).font(FontId::proportional(12.0)));
                });
            });
        }

        if self.show_login { self.show_login_popup(ctx); }

        let tab = self.tab.clone();
        match tab {
            Tab::Library => self.show_library(ctx),
            Tab::Friends => self.show_friends(ctx),
            Tab::Settings => self.show_settings(ctx),
        }

        if !self.pending_imgs.lock().unwrap().is_empty() || *self.loading.lock().unwrap() || *self.resolving_names.lock().unwrap() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    // ===================== LIBRARY =====================

    fn show_library(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("lib_top").frame(egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(18.0, 10.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                egui::Frame::none().fill(Color32::from_rgb(13, 16, 22)).rounding(Rounding::same(5.0)).stroke(Stroke::new(1.0, C_BORDER_LIT)).inner_margin(egui::Margin::symmetric(10.0, 5.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("🔍").font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                        ui.add_space(4.0);
                        ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(200.0).hint_text("Buscar juego...").frame(false).text_color(C_TEXT));
                    });
                });

                ui.add_space(10.0);

                // Filtros
                for (label, active) in [("Todos", !self.show_installed_only), ("Instalados", self.show_installed_only)] {
                    let btn = ui.add(egui::Button::new(RichText::new(label).font(FontId::proportional(12.0)).color(if active { C_ACCENT } else { C_TEXT_DIM }))
                        .fill(if active { Color32::from_rgb(22, 40, 60) } else { Color32::TRANSPARENT })
                        .rounding(Rounding::same(4.0)));
                    if btn.clicked() { self.show_installed_only = label == "Instalados"; }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("↺").font(FontId::proportional(14.0)).color(C_TEXT_DIM)).fill(Color32::TRANSPARENT).rounding(Rounding::same(4.0))).clicked() { self.reload(); }
                });
            });
        });

        let games_snap: Vec<Game> = {
            let gs = self.games.lock().unwrap();
            let q = self.search.to_lowercase();
            gs.iter().filter(|g| {
                let name_ok = q.is_empty() || g.name.to_lowercase().contains(&q);
                let filter_ok = !self.show_installed_only || g.installed;
                name_ok && filter_ok
            }).cloned().collect()
        };

        // Precargar solo primeras 40 imágenes
        for g in games_snap.iter().take(40) {
            if !self.textures.contains_key(&g.appid) { self.req_img(g.appid, g.img_url()); }
        }

        let mut launch: Option<Game> = None;
        let mut install_id: Option<u64> = None;
        let mut open_url: Option<String> = None;

        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            if games_snap.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("No se encontraron juegos").font(FontId::proportional(15.0)).color(C_TEXT_DIM));
                });
                return;
            }

            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(14.0);

                let sp = 10.0_f32;
                let padding = 28.0_f32;
                let available_w = ui.available_width() - padding;
                let cols = ((available_w + sp) / (CARD_W + sp)).floor().max(1.0) as usize;
                // Centrar el grid
                let total_w = cols as f32 * CARD_W + (cols - 1) as f32 * sp;
                let left_margin = ((available_w - total_w) / 2.0).max(14.0);

                ui.horizontal(|ui| {
                    ui.add_space(left_margin);
                    egui::Grid::new("grid")
                        .num_columns(cols)
                        .spacing(Vec2::splat(sp))
                        .min_col_width(CARD_W)
                        .max_col_width(CARD_W)
                        .show(ui, |ui| {
                            for (i, g) in games_snap.iter().enumerate() {
                                if i > 0 && i % cols == 0 { ui.end_row(); }

                                // Card con altura FIJA
                                egui::Frame::none()
                                    .fill(C_CARD)
                                    .rounding(Rounding::same(6.0))
                                    .stroke(Stroke::new(1.0, if g.installed { C_BORDER_LIT } else { C_BORDER }))
                                    .show(ui, |ui| {
                                        ui.set_width(CARD_W);
                                        ui.set_min_height(CARD_H);
                                        ui.set_max_height(CARD_H);

                                        // Imagen con altura FIJA siempre
                                        if let Some(tex) = self.textures.get(&g.appid) {
                                            let img = egui::Image::new(tex)
                                                .fit_to_exact_size(Vec2::new(CARD_W, CARD_IMG_H))
                                                .rounding(Rounding { nw: 5.0, ne: 5.0, sw: 0.0, se: 0.0 });
                                            ui.add(img);
                                        } else {
                                            // Placeholder mismo tamaño siempre
                                            let (rect, _) = ui.allocate_exact_size(Vec2::new(CARD_W, CARD_IMG_H), egui::Sense::hover());
                                            ui.painter().rect_filled(rect, Rounding { nw: 5.0, ne: 5.0, sw: 0.0, se: 0.0 }, Color32::from_rgb(18, 22, 32));
                                            // Spinner si está cargando
                                            let t = ui.input(|i| i.time);
                                            let angle = (t * 2.0) as f32;
                                            for k in 0..8 {
                                                let a = angle + k as f32 * std::f32::consts::PI / 4.0;
                                                let alpha = ((k as f32 / 8.0) * 180.0) as u8;
                                                let off = Pos2::new(rect.center().x + a.cos() * 8.0, rect.center().y + a.sin() * 8.0);
                                                ui.painter().circle_filled(off, 2.0, Color32::from_rgba_unmultiplied(102, 192, 244, alpha));
                                            }
                                        }

                                        // Info — altura fija también
                                        egui::Frame::none().inner_margin(egui::Margin::symmetric(9.0, 7.0)).show(ui, |ui| {
                                            ui.set_width(CARD_W - 18.0);
                                            ui.set_max_height(CARD_INFO_H - 14.0);

                                            // Nombre (máx 2 líneas)
                                            let name = if g.name.len() > 28 { format!("{}...", &g.name[..25]) } else { g.name.clone() };
                                            ui.label(RichText::new(&name).font(FontId::proportional(11.5)).color(C_TEXT).strong());

                                            ui.horizontal(|ui| {
                                                if g.installed {
                                                    let (r, _) = ui.allocate_exact_size(Vec2::new(6.0, 6.0), egui::Sense::hover());
                                                    ui.painter().circle_filled(r.center(), 3.0, C_GREEN);
                                                    ui.add_space(2.0);
                                                    ui.label(RichText::new("Instalado").font(FontId::proportional(10.0)).color(C_GREEN));
                                                } else {
                                                    ui.label(RichText::new(g.playtime_str()).font(FontId::proportional(10.0)).color(C_TEXT_DIM));
                                                }
                                            });

                                            ui.add_space(5.0);

                                            ui.horizontal(|ui| {
                                                if g.installed {
                                                    let play = ui.add(egui::Button::new(RichText::new("▶  Jugar").font(FontId::proportional(10.5)).color(Color32::WHITE).strong()).fill(C_BTN).rounding(Rounding::same(4.0)).min_size(Vec2::new(76.0, 22.0)));
                                                    if play.clicked() { launch = Some(g.clone()); }
                                                } else {
                                                    let inst = ui.add(egui::Button::new(RichText::new("⬇  Instalar").font(FontId::proportional(10.0)).color(Color32::WHITE).strong()).fill(C_BTN_GREEN).rounding(Rounding::same(4.0)).min_size(Vec2::new(76.0, 22.0)));
                                                    if inst.clicked() { install_id = Some(g.appid); }
                                                }
                                                ui.add_space(3.0);
                                                let more = ui.add(egui::Button::new(RichText::new("···").font(FontId::proportional(12.0)).color(C_TEXT_DIM)).fill(Color32::from_rgb(26, 33, 46)).rounding(Rounding::same(4.0)).min_size(Vec2::new(24.0, 22.0)));
                                                if more.clicked() { open_url = Some(g.store_url()); }
                                            });
                                        });
                                    });
                            }
                        });
                });
                ui.add_space(20.0);
            });
        });

        if let Some(g) = launch { open::that(g.launch_url()).ok(); self.last_played = Some(g.name); ctx.request_repaint(); }
        if let Some(id) = install_id { self.do_download(id); }
        if let Some(url) = open_url { open::that(url).ok(); }
    }

    // ===================== FRIENDS =====================

    fn show_friends(&mut self, ctx: &egui::Context) {
        let friends = self.friends.lock().unwrap().clone();

        egui::TopBottomPanel::top("fr_top").frame(egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(18.0, 12.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Amigos").font(FontId::proportional(15.0)).color(C_TEXT).strong());
                ui.add_space(10.0);
                let online = friends.iter().filter(|f| f.state > 0).count();
                ui.label(RichText::new(format!("{} en línea  ·  {} total", online, friends.len())).font(FontId::proportional(12.0)).color(C_TEXT_DIM));
            });
        });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            if friends.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(RichText::new("👥").font(FontId::proportional(40.0)));
                        ui.add_space(12.0);
                        ui.label(RichText::new("No se encontraron amigos").font(FontId::proportional(15.0)).color(C_TEXT_DIM));
                        ui.add_space(6.0);
                        ui.label(RichText::new("Abre Steam una vez para sincronizar la lista.").font(FontId::proportional(12.0)).color(C_TEXT_FAINT));
                    });
                });
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(12.0);

                let online: Vec<&Friend> = friends.iter().filter(|f| f.state > 0).collect();
                let offline: Vec<&Friend> = friends.iter().filter(|f| f.state == 0).collect();

                if !online.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(RichText::new(format!("EN LÍNEA — {}", online.len())).font(FontId::proportional(10.0)).color(C_TEXT_FAINT).strong());
                    });
                    ui.add_space(6.0);
                    for f in &online { self.draw_friend(ui, f); }
                    ui.add_space(14.0);
                }

                if !offline.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(RichText::new(format!("DESCONECTADO — {}", offline.len())).font(FontId::proportional(10.0)).color(C_TEXT_FAINT).strong());
                    });
                    ui.add_space(6.0);
                    for f in &offline { self.draw_friend(ui, f); }
                }

                ui.add_space(16.0);
            });
        });
    }

    fn draw_friend(&self, ui: &mut egui::Ui, f: &Friend) {
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            egui::Frame::none().fill(C_CARD).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(12.0, 9.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
                ui.set_min_width(ui.available_width() - 16.0);
                ui.horizontal(|ui| {
                    // Avatar con inicial
                    let (r, _) = ui.allocate_exact_size(Vec2::new(34.0, 34.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, Rounding::same(4.0), Color32::from_rgb(28, 38, 54));
                    let init = f.name.chars().next().unwrap_or('?').to_uppercase().to_string();
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, &init, FontId::proportional(14.0), C_TEXT_DIM);
                    // Punto estado
                    ui.painter().circle_filled(Pos2::new(r.max.x - 3.0, r.max.y - 3.0), 5.0, C_BG);
                    ui.painter().circle_filled(Pos2::new(r.max.x - 3.0, r.max.y - 3.0), 4.0, f.state_color());

                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&f.name).font(FontId::proportional(13.0)).color(C_TEXT).strong());
                        if let Some(g) = &f.game {
                            ui.label(RichText::new(format!("▶  {}", g)).font(FontId::proportional(11.0)).color(C_GREEN));
                        } else {
                            ui.label(RichText::new(f.status_label()).font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                        }
                    });
                });
            });
        });
        ui.add_space(3.0);
    }

    // ===================== SETTINGS =====================

    fn show_settings(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("set_top").frame(egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(18.0, 12.0))).show(ctx, |ui| {
            ui.label(RichText::new("Ajustes").font(FontId::proportional(15.0)).color(C_TEXT).strong());
        });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    ui.vertical(|ui| {
                        ui.set_max_width(520.0);

                        // SteamCMD
                        egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(7.0)).inner_margin(egui::Margin::same(18.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
                            ui.set_max_width(520.0);
                            ui.label(RichText::new("SteamCMD — Descarga directa").font(FontId::proportional(13.0)).color(C_TEXT).strong());
                            ui.add_space(12.0);
                            ui.label(RichText::new("Usuario de Steam").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                            ui.add_space(3.0);
                            ui.add(egui::TextEdit::singleline(&mut self.steamcmd_user).desired_width(300.0).hint_text("usuario_steam").text_color(C_TEXT));
                            ui.add_space(10.0);
                            ui.label(RichText::new("Contraseña").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                            ui.add_space(3.0);
                            ui.add(egui::TextEdit::singleline(&mut self.steamcmd_pass).desired_width(300.0).password(true).hint_text("••••••••").text_color(C_TEXT));
                            ui.add_space(10.0);
                            ui.label(RichText::new("Steam Guard (si lo pide)").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                            ui.add_space(3.0);
                            ui.add(egui::TextEdit::singleline(&mut self.steamcmd_guard).desired_width(200.0).hint_text("Déjalo vacío si no lo pide").text_color(C_TEXT));
                        });

                        ui.add_space(12.0);

                        // Acciones
                        egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(7.0)).inner_margin(egui::Margin::same(18.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
                            ui.set_max_width(520.0);
                            ui.label(RichText::new("Acciones").font(FontId::proportional(13.0)).color(C_TEXT).strong());
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new(RichText::new("↺  Recargar").font(FontId::proportional(12.0)).color(C_TEXT)).fill(Color32::from_rgb(28, 38, 55)).rounding(Rounding::same(5.0)).min_size(Vec2::new(120.0, 30.0))).clicked() { self.reload(); }
                                ui.add_space(8.0);
                                if ui.add(egui::Button::new(RichText::new("Abrir Steam").font(FontId::proportional(12.0)).color(C_TEXT)).fill(Color32::from_rgb(28, 38, 55)).rounding(Rounding::same(5.0)).min_size(Vec2::new(110.0, 30.0))).clicked() {
                                    Command::new("cmd").args(["/C", "start", "", r"C:\Program Files (x86)\Steam\steam.exe", "-no-browser", "-silent"]).spawn().ok();
                                }
                            });
                            ui.add_space(10.0);
                            if let Some(cfg) = &self.config {
                                ui.label(RichText::new(format!("Steam: {}", cfg.steam_path)).font(FontId::proportional(11.0)).color(C_TEXT_FAINT));
                            }
                        });

                        ui.add_space(12.0);

                        // Zona peligro
                        egui::Frame::none().fill(Color32::from_rgb(26, 16, 16)).rounding(Rounding::same(7.0)).inner_margin(egui::Margin::same(18.0)).stroke(Stroke::new(1.0, Color32::from_rgb(70, 25, 25))).show(ui, |ui| {
                            ui.set_max_width(520.0);
                            ui.label(RichText::new("Zona de peligro").font(FontId::proportional(13.0)).color(C_RED).strong());
                            ui.add_space(10.0);
                            if ui.add(egui::Button::new(RichText::new("Resetear configuración").font(FontId::proportional(12.0)).color(C_RED)).fill(Color32::from_rgb(40, 12, 12)).rounding(Rounding::same(5.0)).min_size(Vec2::new(180.0, 30.0))).clicked() {
                                delete_config(); self.config = None; self.screen = Screen::Setup; *self.games.lock().unwrap() = vec![];
                            }
                        });
                    });
                });
                ui.add_space(20.0);
            });
        });
    }

    // ===================== LOGIN POPUP =====================

    fn show_login_popup(&mut self, ctx: &egui::Context) {
        let mut open = self.show_login;
        egui::Window::new("Iniciar sesión para descargar").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).open(&mut open)
            .frame(egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(10.0)).stroke(Stroke::new(1.0, C_BORDER)).inner_margin(egui::Margin::same(22.0)))
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label(RichText::new("SteamCMD descargará el juego en segundo plano.").font(FontId::proportional(13.0)).color(C_TEXT));
                ui.add_space(4.0);
                ui.label(RichText::new("Tus credenciales se guardan solo en tu PC.").font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                ui.add_space(16.0);
                ui.label(RichText::new("Usuario").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                ui.add_space(3.0);
                ui.add(egui::TextEdit::singleline(&mut self.steamcmd_user).desired_width(320.0).text_color(C_TEXT));
                ui.add_space(10.0);
                ui.label(RichText::new("Contraseña").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                ui.add_space(3.0);
                ui.add(egui::TextEdit::singleline(&mut self.steamcmd_pass).desired_width(320.0).password(true).text_color(C_TEXT));
                ui.add_space(10.0);
                ui.label(RichText::new("Steam Guard (opcional)").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                ui.add_space(3.0);
                ui.add(egui::TextEdit::singleline(&mut self.steamcmd_guard).desired_width(200.0).hint_text("Déjalo vacío").text_color(C_TEXT));
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("⬇  Descargar").font(FontId::proportional(12.0)).color(Color32::WHITE).strong()).fill(C_BTN_GREEN).rounding(Rounding::same(5.0)).min_size(Vec2::new(120.0, 32.0))).clicked() {
                        if let Some(id) = self.pending_appid { self.show_login = false; self.do_download(id); self.pending_appid = None; }
                    }
                    ui.add_space(8.0);
                    if ui.add(egui::Button::new(RichText::new("Cancelar").font(FontId::proportional(12.0)).color(C_TEXT_DIM)).fill(Color32::from_rgb(28, 35, 48)).rounding(Rounding::same(5.0)).min_size(Vec2::new(90.0, 32.0))).clicked() {
                        self.show_login = false;
                        if let Some(id) = self.pending_appid { open::that(format!("steam://install/{}", id)).ok(); self.pending_appid = None; }
                    }
                });
            });
        if !open { self.show_login = false; }
    }
}

// ===================== APP LOOP =====================

impl eframe::App for SteamLite {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut style = (*ctx.style()).clone();
        style.visuals.dark_mode = true;
        style.visuals.panel_fill = C_BG;
        style.visuals.window_fill = C_PANEL;
        style.visuals.window_rounding = Rounding::same(8.0);
        style.visuals.window_shadow = egui::epaint::Shadow { blur: 20.0, spread: 0.0, offset: [0.0, 4.0].into(), color: Color32::from_black_alpha(80) };
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(26, 33, 46);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(35, 46, 65);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(42, 56, 80);
        style.visuals.widgets.inactive.rounding = Rounding::same(4.0);
        style.visuals.widgets.hovered.rounding = Rounding::same(4.0);
        style.visuals.selection.bg_fill = C_BTN;
        style.spacing.item_spacing = Vec2::new(6.0, 4.0);
        style.spacing.button_padding = Vec2::new(8.0, 4.0);
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
        viewport: egui::ViewportBuilder::default()
            .with_title("Steam Lite")
            .with_inner_size([1150.0, 720.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    }, Box::new(|cc| Box::new(SteamLite::new(cc))))
}
