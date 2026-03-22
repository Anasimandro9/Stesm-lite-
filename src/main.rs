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

// ===================== PALETA "STEAM REFINED" =====================
const C_BG:           Color32 = Color32::from_rgb(16, 18, 24);      // Fondo principal
const C_SIDEBAR:      Color32 = Color32::from_rgb(11, 13, 18);      // Sidebar
const C_PANEL:        Color32 = Color32::from_rgb(20, 24, 32);      // Panels secundarios
const C_CARD:         Color32 = Color32::from_rgb(24, 29, 40);      // Cards
const C_CARD_HOVER:   Color32 = Color32::from_rgb(30, 38, 52);      // Card hover
const C_TOPBAR:       Color32 = Color32::from_rgb(13, 15, 21);      // Barra superior
const C_BORDER:       Color32 = Color32::from_rgb(38, 48, 65);      // Bordes sutiles
const C_BORDER_LIT:   Color32 = Color32::from_rgb(55, 75, 105);     // Bordes activos
const C_ACCENT:       Color32 = Color32::from_rgb(102, 192, 244);   // Azul Steam
const C_ACCENT_DIM:   Color32 = Color32::from_rgb(60, 130, 180);    // Azul atenuado
const C_GREEN:        Color32 = Color32::from_rgb(74, 197, 90);     // Verde instalado
const C_GREEN_DIM:    Color32 = Color32::from_rgb(45, 130, 55);     // Verde botón
const C_TEXT:         Color32 = Color32::from_rgb(195, 208, 220);   // Texto principal
const C_TEXT_DIM:     Color32 = Color32::from_rgb(110, 130, 155);   // Texto secundario
const C_TEXT_FAINT:   Color32 = Color32::from_rgb(65, 80, 100);     // Texto muy tenue
const C_BTN:          Color32 = Color32::from_rgb(55, 125, 170);    // Botón primario
const C_BTN_HOVER:    Color32 = Color32::from_rgb(70, 150, 200);    // Botón hover
const C_BTN_GREEN:    Color32 = Color32::from_rgb(50, 145, 60);     // Botón instalar
const C_RED:          Color32 = Color32::from_rgb(200, 70, 70);     // Error/peligro

// ===================== CONFIG =====================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    steam_path: String,
    steam_user: Option<String>,
    api_key: Option<String>,
}

fn config_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.pop(); p.push("steamlite_config.json"); p
}

fn load_config() -> Option<Config> {
    serde_json::from_str(&fs::read_to_string(config_path()).ok()?).ok()
}

fn save_config(c: &Config) {
    if let Ok(j) = serde_json::to_string_pretty(c) { fs::write(config_path(), j).ok(); }
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
        else { format!("{:.1}h", h) }
    }
    fn img_url(&self) -> String { format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{}/header.jpg", self.appid) }
    fn launch_url(&self) -> String { format!("steam://run/{}", self.appid) }
    fn store_url(&self) -> String { format!("https://store.steampowered.com/app/{}", self.appid) }
}

#[derive(Debug, Clone)]
struct Friend { name: String, state: u8, game: Option<String> }

impl Friend {
    fn status_label(&self) -> &str { match self.state { 1 => "En línea", 2 => "Ocupado", 3 => "Ausente", _ => "Desconectado" } }
    fn state_color(&self) -> Color32 { match self.state { 1 => C_GREEN, 2 => Color32::from_rgb(200,80,80), 3 => Color32::from_rgb(200,160,40), _ => C_TEXT_FAINT } }
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

fn load_games(path: &str) -> Result<Vec<Game>, String> {
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
            let name = match acf_val(&c, "name") { Some(n) => n, _ => continue };
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
                                all.insert(id, Game { appid: id, name: format!("App {}", id), playtime_forever: cur_pt, installed: false });
                            }
                        }
                        cur_id = None; cur_pt = 0;
                    }
                    depth -= 1; if depth <= 0 { break; }
                } else if depth == 1 { if let Ok(id) = t.trim_matches('"').parse::<u64>() { cur_id = Some(id); } }
                else if depth == 2 && t.to_lowercase().contains("\"playtime\"") { cur_pt = t.split('"').nth(3).unwrap_or("0").parse().unwrap_or(0); }
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

fn fetch_img(url: &str) -> Option<Vec<u8>> {
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
#[derive(PartialEq)] enum LibraryView { Grid, List }

struct SteamLite {
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
    lib_view: LibraryView,
    selected_game: Option<u64>,

    textures: HashMap<u64, egui::TextureHandle>,
    pending_imgs: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
    fetching: Arc<Mutex<HashSet<u64>>>,
    last_played: Option<String>,

    steamcmd_user: String,
    steamcmd_pass: String,
    steamcmd_guard: String,
    show_login: bool,
    pending_appid: Option<u64>,
    input_api_key: String,
}

impl SteamLite {
    fn new(_cc: &eframe::CreationContext) -> Self {
        let config = load_config();
        let screen = if config.is_some() { Screen::Main } else { Screen::Setup };
        let auto = find_steam().unwrap_or_default();
        let user = config.as_ref().and_then(|c| c.steam_user.clone()).unwrap_or_default();
        let api = config.as_ref().and_then(|c| c.api_key.clone()).unwrap_or_default();
        Self {
            screen, config, input_path: auto.clone(), setup_error: String::new(), autodetected: !auto.is_empty(),
            games: Arc::new(Mutex::new(vec![])), friends: Arc::new(Mutex::new(vec![])),
            loading: Arc::new(Mutex::new(false)), load_error: Arc::new(Mutex::new(String::new())), dl_status: Arc::new(Mutex::new(String::new())),
            search: String::new(), tab: Tab::Library, lib_view: LibraryView::Grid, selected_game: None,
            textures: HashMap::new(), pending_imgs: Arc::new(Mutex::new(vec![])), fetching: Arc::new(Mutex::new(HashSet::new())), last_played: None,
            steamcmd_user: user, steamcmd_pass: String::new(), steamcmd_guard: String::new(), show_login: false, pending_appid: None, input_api_key: api,
        }
    }

    fn reload(&self) {
        let cfg = match &self.config { Some(c) => c.clone(), None => return };
        let (games, friends, loading, error) = (Arc::clone(&self.games), Arc::clone(&self.friends), Arc::clone(&self.loading), Arc::clone(&self.load_error));
        *loading.lock().unwrap() = true; *error.lock().unwrap() = String::new();
        thread::spawn(move || {
            match load_games(&cfg.steam_path) { Ok(g) => *games.lock().unwrap() = g, Err(e) => *error.lock().unwrap() = e }
            *friends.lock().unwrap() = load_friends(&cfg.steam_path);
            *loading.lock().unwrap() = false;
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
        let cfg = Config { steam_path: path, steam_user: None, api_key: None };
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
            // Fondo con efecto sutil
            let rect = ui.max_rect();
            ui.painter().rect_filled(Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + 3.0)), Rounding::ZERO, C_ACCENT_DIM);

            ui.add_space(80.0);
            ui.vertical_centered(|ui| {
                // Logo
                ui.label(RichText::new("STEAM LITE").font(FontId::proportional(46.0)).color(C_ACCENT).strong());
                ui.add_space(6.0);
                ui.label(RichText::new("Cliente refinado · Bajo consumo · Sin compromiso").font(FontId::proportional(13.0)).color(C_TEXT_DIM));
                ui.add_space(50.0);

                egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(10.0)).inner_margin(egui::Margin::same(32.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
                    ui.set_max_width(460.0);

                    ui.label(RichText::new("Configuración inicial").font(FontId::proportional(17.0)).color(C_TEXT).strong());
                    ui.add_space(4.0);
                    ui.label(RichText::new("Solo necesitas hacerlo una vez.").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                    ui.add_space(22.0);

                    if self.autodetected {
                        egui::Frame::none().fill(Color32::from_rgb(18, 45, 22)).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(12.0, 7.0)).stroke(Stroke::new(1.0, C_GREEN_DIM)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("✓").color(C_GREEN).font(FontId::proportional(13.0)));
                                ui.add_space(4.0);
                                ui.label(RichText::new("Steam detectado automáticamente").font(FontId::proportional(12.0)).color(C_GREEN));
                            });
                        });
                        ui.add_space(14.0);
                    }

                    ui.label(RichText::new("Ruta de instalación de Steam").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                    ui.add_space(4.0);
                    egui::Frame::none().fill(Color32::from_rgb(13, 16, 22)).rounding(Rounding::same(5.0)).stroke(Stroke::new(1.0, C_BORDER_LIT)).inner_margin(egui::Margin::symmetric(10.0, 6.0)).show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.input_path).desired_width(400.0).hint_text(r"C:\Program Files (x86)\Steam").frame(false).text_color(C_TEXT));
                    });

                    if !self.setup_error.is_empty() {
                        ui.add_space(10.0);
                        egui::Frame::none().fill(Color32::from_rgb(45, 15, 15)).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(12.0, 7.0)).show(ui, |ui| {
                            ui.label(RichText::new(&self.setup_error).color(C_RED).font(FontId::proportional(12.0)));
                        });
                    }

                    ui.add_space(20.0);
                    let btn = ui.add_sized(Vec2::new(424.0, 40.0), egui::Button::new(RichText::new("Entrar →").font(FontId::proportional(14.0)).color(Color32::WHITE).strong()).fill(C_BTN).rounding(Rounding::same(5.0)));
                    if btn.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                    if btn.clicked() { self.do_setup(); }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) { self.do_setup(); }
                });

                ui.add_space(20.0);
                ui.label(RichText::new("🔒  Tus datos permanecen en tu PC. Sin telemetría.").font(FontId::proportional(11.0)).color(C_TEXT_FAINT));
            });
        });
    }
}

// ===================== MAIN =====================

impl SteamLite {
    fn show_main(&mut self, ctx: &egui::Context) {
        // Procesar texturas
        {
            let mut p = self.pending_imgs.lock().unwrap();
            for (id, bytes) in p.drain(..) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    let img = img.to_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    let pixels = img.pixels().map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
                    let tex = ctx.load_texture(format!("g{}", id), egui::ColorImage { size, pixels }, egui::TextureOptions::LINEAR);
                    self.textures.insert(id, tex);
                }
            }
        }

        // SIDEBAR
        egui::SidePanel::left("sidebar").exact_width(220.0)
            .frame(egui::Frame::none().fill(C_SIDEBAR))
            .show(ctx, |ui| {
                ui.set_min_height(ui.available_height());

                // Logo area
                egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(18.0, 16.0)).show(ui, |ui| {
                    ui.set_min_width(220.0);
                    ui.label(RichText::new("STEAM LITE").font(FontId::proportional(17.0)).color(C_ACCENT).strong());
                    ui.add_space(2.0);
                    let cnt = self.games.lock().unwrap().len();
                    let loading = *self.loading.lock().unwrap();
                    if loading {
                        ui.label(RichText::new("Cargando...").font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                    } else {
                        ui.label(RichText::new(format!("{} juegos en biblioteca", cnt)).font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                    }
                });

                ui.add_space(8.0);

                // Nav
                let nav_items = [("  🎮  Biblioteca", Tab::Library), ("  👥  Amigos", Tab::Friends), ("  ⚙   Ajustes", Tab::Settings)];
                for (label, t) in &nav_items {
                    let selected = self.tab == *t;
                    let response = ui.add_sized(
                        Vec2::new(220.0, 36.0),
                        egui::Button::new(RichText::new(*label).font(FontId::proportional(13.0)).color(if selected { C_ACCENT } else { C_TEXT }))
                            .fill(if selected { Color32::from_rgb(25, 40, 60) } else { Color32::TRANSPARENT })
                            .rounding(Rounding::ZERO)
                            .frame(true),
                    );
                    if selected {
                        let r = response.rect;
                        ui.painter().rect_filled(Rect::from_min_max(r.min, Pos2::new(r.min.x + 3.0, r.max.y)), Rounding::ZERO, C_ACCENT);
                    }
                    if response.clicked() { self.tab = t.clone(); }
                }

                ui.add_space(16.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(12.0);

                // Status
                let ds = self.dl_status.lock().unwrap().clone();
                if !ds.is_empty() {
                    egui::Frame::none().inner_margin(egui::Margin::symmetric(14.0, 0.0)).show(ui, |ui| {
                        ui.label(RichText::new(&ds).font(FontId::proportional(11.0)).color(C_GREEN));
                    });
                } else if let Some(name) = &self.last_played.clone() {
                    egui::Frame::none().inner_margin(egui::Margin::symmetric(14.0, 0.0)).show(ui, |ui| {
                        ui.label(RichText::new("En juego").font(FontId::proportional(10.0)).color(C_TEXT_DIM));
                        ui.label(RichText::new(name).font(FontId::proportional(12.0)).color(C_GREEN).strong());
                    });
                }

                // Bottom - cerrar sesion
                let available = ui.available_height();
                if available > 40.0 { ui.add_space(available - 40.0); }
                egui::Frame::none().inner_margin(egui::Margin::symmetric(14.0, 8.0)).show(ui, |ui| {
                    if ui.add(egui::Button::new(RichText::new("Cerrar sesión").font(FontId::proportional(11.0)).color(C_TEXT_DIM)).fill(Color32::TRANSPARENT).rounding(Rounding::same(4.0))).clicked() {
                        delete_config(); self.config = None; self.screen = Screen::Setup; *self.games.lock().unwrap() = vec![];
                    }
                });
            });

        // Error banner
        let error = self.load_error.lock().unwrap().clone();
        if !error.is_empty() {
            egui::TopBottomPanel::top("err").frame(egui::Frame::none().fill(Color32::from_rgb(90, 18, 18)).inner_margin(egui::Margin::symmetric(16.0, 8.0))).show(ctx, |ui| {
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

        if !self.pending_imgs.lock().unwrap().is_empty() || *self.loading.lock().unwrap() { ctx.request_repaint(); }
    }

    // ===================== LIBRARY =====================

    fn show_library(&mut self, ctx: &egui::Context) {
        // Topbar de biblioteca
        egui::TopBottomPanel::top("lib_top").frame(egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(18.0, 10.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Searchbox
                egui::Frame::none().fill(Color32::from_rgb(13, 16, 22)).rounding(Rounding::same(5.0)).stroke(Stroke::new(1.0, C_BORDER_LIT)).inner_margin(egui::Margin::symmetric(10.0, 5.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("🔍").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                        ui.add_space(4.0);
                        ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(220.0).hint_text("Buscar juego...").frame(false).text_color(C_TEXT));
                    });
                });

                ui.add_space(12.0);

                // Filtros rápidos
                for (label, active) in [("Todos", true), ("Instalados", false)] {
                    let fg = if active { C_ACCENT } else { C_TEXT_DIM };
                    let bg = if active { Color32::from_rgb(25, 45, 65) } else { Color32::TRANSPARENT };
                    ui.add(egui::Button::new(RichText::new(label).font(FontId::proportional(12.0)).color(fg)).fill(bg).rounding(Rounding::same(4.0)));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Toggle vista
                    let grid_active = self.lib_view == LibraryView::Grid;
                    if ui.add(egui::Button::new(RichText::new("▦").font(FontId::proportional(14.0)).color(if grid_active { C_ACCENT } else { C_TEXT_DIM })).fill(if grid_active { Color32::from_rgb(25, 45, 65) } else { Color32::TRANSPARENT }).rounding(Rounding::same(4.0))).clicked() { self.lib_view = LibraryView::Grid; }
                    if ui.add(egui::Button::new(RichText::new("≡").font(FontId::proportional(16.0)).color(if !grid_active { C_ACCENT } else { C_TEXT_DIM })).fill(if !grid_active { Color32::from_rgb(25, 45, 65) } else { Color32::TRANSPARENT }).rounding(Rounding::same(4.0))).clicked() { self.lib_view = LibraryView::List; }
                    ui.add_space(8.0);
                    if ui.add(egui::Button::new(RichText::new("↺").font(FontId::proportional(14.0)).color(C_TEXT_DIM)).fill(Color32::TRANSPARENT).rounding(Rounding::same(4.0))).clicked() { self.reload(); }
                });
            });
        });

        let games_snap: Vec<Game> = {
            let gs = self.games.lock().unwrap();
            let q = self.search.to_lowercase();
            gs.iter().filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q)).cloned().collect()
        };

        // Precargar imágenes
        for g in games_snap.iter().take(40) { if !self.textures.contains_key(&g.appid) { self.req_img(g.appid, g.img_url()); } }

        let mut launch: Option<Game> = None;
        let mut install_id: Option<u64> = None;
        let mut open_url: Option<String> = None;

        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            if games_snap.is_empty() && !*self.loading.lock().unwrap() {
                ui.centered_and_justified(|ui| { ui.label(RichText::new("No se encontraron juegos").font(FontId::proportional(16.0)).color(C_TEXT_DIM)); });
                return;
            }

            match self.lib_view {
                LibraryView::Grid => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(14.0);
                        let cw = 210.0_f32;
                        let ch = 98.0_f32;
                        let sp = 10.0_f32;
                        let cols = ((ui.available_width() - 28.0 + sp) / (cw + sp)).floor().max(1.0) as usize;

                        ui.horizontal(|ui| { ui.add_space(14.0); });

                        egui::Grid::new("grid").num_columns(cols).spacing(Vec2::splat(sp)).min_col_width(cw).show(ui, |ui| {
                            for (i, g) in games_snap.iter().enumerate() {
                                if i > 0 && i % cols == 0 { ui.end_row(); }

                                let selected = self.selected_game == Some(g.appid);
                                let border = if selected { C_ACCENT } else if g.installed { C_BORDER_LIT } else { C_BORDER };

                                let response = egui::Frame::none().fill(C_CARD).rounding(Rounding::same(6.0)).stroke(Stroke::new(1.0, border)).show(ui, |ui| {
                                    ui.set_max_width(cw);

                                    // Imagen
                                    if let Some(tex) = self.textures.get(&g.appid) {
                                        ui.add(egui::Image::new(tex).max_width(cw).max_height(ch).rounding(Rounding { nw: 5.0, ne: 5.0, sw: 0.0, se: 0.0 }));
                                    } else {
                                        let (rect, _) = ui.allocate_exact_size(Vec2::new(cw, ch), egui::Sense::hover());
                                        ui.painter().rect_filled(rect, Rounding { nw: 5.0, ne: 5.0, sw: 0.0, se: 0.0 }, Color32::from_rgb(18, 22, 32));
                                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &g.name, FontId::proportional(9.5), C_TEXT_FAINT);
                                    }

                                    // Info
                                    egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 6.0)).show(ui, |ui| {
                                        // Nombre truncado
                                        let name = if g.name.len() > 26 { format!("{}...", &g.name[..23]) } else { g.name.clone() };
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
                                                let play = ui.add(egui::Button::new(RichText::new("▶  Jugar").font(FontId::proportional(11.0)).color(Color32::WHITE).strong()).fill(C_BTN).rounding(Rounding::same(4.0)).min_size(Vec2::new(82.0, 24.0)));
                                                if play.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                                                if play.clicked() { launch = Some(g.clone()); }
                                            } else {
                                                let inst = ui.add(egui::Button::new(RichText::new("⬇  Instalar").font(FontId::proportional(10.5)).color(Color32::WHITE).strong()).fill(C_BTN_GREEN).rounding(Rounding::same(4.0)).min_size(Vec2::new(82.0, 24.0)));
                                                if inst.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                                                if inst.clicked() { install_id = Some(g.appid); }
                                            }
                                            ui.add_space(4.0);
                                            let store = ui.add(egui::Button::new(RichText::new("···").font(FontId::proportional(13.0)).color(C_TEXT_DIM)).fill(Color32::from_rgb(28, 35, 48)).rounding(Rounding::same(4.0)).min_size(Vec2::new(26.0, 24.0)));
                                            if store.clicked() { open_url = Some(g.store_url()); }
                                        });
                                        ui.add_space(4.0);
                                    });
                                });

                                if response.response.interact(egui::Sense::click()).clicked() {
                                    self.selected_game = Some(g.appid);
                                }
                            }
                        });
                        ui.add_space(20.0);
                    });
                }
                LibraryView::List => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(8.0);
                        for g in &games_snap {
                            let selected = self.selected_game == Some(g.appid);
                            let bg = if selected { C_CARD_HOVER } else { C_CARD };

                            let resp = egui::Frame::none().fill(bg).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(14.0, 8.0)).stroke(Stroke::new(1.0, if selected { C_BORDER_LIT } else { C_BORDER })).show(ui, |ui| {
                                ui.set_min_width(ui.available_width() - 28.0);
                                ui.horizontal(|ui| {
                                    // Miniatura
                                    if let Some(tex) = self.textures.get(&g.appid) {
                                        ui.add(egui::Image::new(tex).max_width(72.0).max_height(34.0).rounding(Rounding::same(3.0)));
                                    } else {
                                        let (r, _) = ui.allocate_exact_size(Vec2::new(72.0, 34.0), egui::Sense::hover());
                                        ui.painter().rect_filled(r, Rounding::same(3.0), C_BG);
                                    }
                                    ui.add_space(10.0);

                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(&g.name).font(FontId::proportional(13.0)).color(C_TEXT).strong());
                                        ui.label(RichText::new(g.playtime_str()).font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                                    });

                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if g.installed {
                                            if ui.add(egui::Button::new(RichText::new("▶  Jugar").font(FontId::proportional(11.0)).color(Color32::WHITE)).fill(C_BTN).rounding(Rounding::same(4.0)).min_size(Vec2::new(80.0, 26.0))).clicked() { launch = Some(g.clone()); }
                                        } else {
                                            if ui.add(egui::Button::new(RichText::new("⬇  Instalar").font(FontId::proportional(11.0)).color(Color32::WHITE)).fill(C_BTN_GREEN).rounding(Rounding::same(4.0)).min_size(Vec2::new(80.0, 26.0))).clicked() { install_id = Some(g.appid); }
                                        }
                                        ui.add_space(8.0);
                                        if g.installed {
                                            let (r, _) = ui.allocate_exact_size(Vec2::new(8.0, 8.0), egui::Sense::hover());
                                            ui.painter().circle_filled(r.center(), 4.0, C_GREEN);
                                        }
                                    });
                                });
                            });

                            if resp.response.interact(egui::Sense::click()).clicked() { self.selected_game = Some(g.appid); }
                            ui.add_space(3.0);
                        }
                        ui.add_space(16.0);
                    });
                }
            }
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
                ui.label(RichText::new("Amigos").font(FontId::proportional(16.0)).color(C_TEXT).strong());
                ui.add_space(8.0);
                let online = friends.iter().filter(|f| f.state > 0).count();
                ui.label(RichText::new(format!("{} en línea  ·  {} total", online, friends.len())).font(FontId::proportional(12.0)).color(C_TEXT_DIM));
            });
        });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            if friends.is_empty() {
                ui.centered_and_justified(|ui| { ui.label(RichText::new("Abre Steam una vez para sincronizar la lista de amigos.").font(FontId::proportional(14.0)).color(C_TEXT_DIM)); });
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(10.0);

                // Agrupar: online primero
                let online: Vec<&Friend> = friends.iter().filter(|f| f.state > 0).collect();
                let offline: Vec<&Friend> = friends.iter().filter(|f| f.state == 0).collect();

                if !online.is_empty() {
                    ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new(format!("EN LÍNEA — {}", online.len())).font(FontId::proportional(10.5)).color(C_TEXT_FAINT).strong()); });
                    ui.add_space(6.0);
                    for f in &online { self.draw_friend(ui, f); }
                    ui.add_space(12.0);
                }

                if !offline.is_empty() {
                    ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new(format!("DESCONECTADO — {}", offline.len())).font(FontId::proportional(10.5)).color(C_TEXT_FAINT).strong()); });
                    ui.add_space(6.0);
                    for f in &offline { self.draw_friend(ui, f); }
                }

                ui.add_space(16.0);
            });
        });
    }

    fn draw_friend(&self, ui: &mut egui::Ui, f: &Friend) {
        egui::Frame::none().fill(C_CARD).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(14.0, 9.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
            ui.set_min_width(ui.available_width() - 28.0);
            ui.horizontal(|ui| {
                // Avatar placeholder
                let (r, _) = ui.allocate_exact_size(Vec2::new(32.0, 32.0), egui::Sense::hover());
                ui.painter().rect_filled(r, Rounding::same(4.0), Color32::from_rgb(30, 40, 55));
                let initials = f.name.chars().next().unwrap_or('?').to_uppercase().to_string();
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, &initials, FontId::proportional(14.0), C_TEXT_DIM);

                // Punto de estado sobre el avatar
                ui.painter().circle_filled(Pos2::new(r.max.x - 4.0, r.max.y - 4.0), 5.0, C_BG);
                ui.painter().circle_filled(Pos2::new(r.max.x - 4.0, r.max.y - 4.0), 4.0, f.state_color());

                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&f.name).font(FontId::proportional(13.0)).color(C_TEXT).strong());
                    if let Some(g) = &f.game {
                        ui.label(RichText::new(format!("▶ {}", g)).font(FontId::proportional(11.0)).color(C_GREEN));
                    } else {
                        ui.label(RichText::new(f.status_label()).font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                    }
                });
            });
        });
        ui.add_space(3.0);
    }

    // ===================== SETTINGS =====================

    fn show_settings(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("set_top").frame(egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(18.0, 12.0))).show(ctx, |ui| {
            ui.label(RichText::new("Ajustes").font(FontId::proportional(16.0)).color(C_TEXT).strong());
        });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    ui.vertical(|ui| {
                        ui.set_max_width(520.0);

                        // SteamCMD
                        self.settings_section(ui, "SteamCMD — Descarga directa de juegos", |ui, s| {
                            ui.label(RichText::new("Usuario de Steam").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                            ui.add_space(3.0);
                            ui.add(egui::TextEdit::singleline(&mut s.steamcmd_user).desired_width(320.0).hint_text("usuario_steam").text_color(C_TEXT));
                            ui.add_space(10.0);
                            ui.label(RichText::new("Contraseña").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                            ui.add_space(3.0);
                            ui.add(egui::TextEdit::singleline(&mut s.steamcmd_pass).desired_width(320.0).password(true).hint_text("••••••••").text_color(C_TEXT));
                            ui.add_space(10.0);
                            ui.label(RichText::new("Código Steam Guard (si lo pide)").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                            ui.add_space(3.0);
                            ui.add(egui::TextEdit::singleline(&mut s.steamcmd_guard).desired_width(200.0).hint_text("Déjalo vacío si no lo pide").text_color(C_TEXT));
                        });

                        ui.add_space(14.0);

                        // Acciones
                        self.settings_section(ui, "Acciones", |ui, s| {
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new(RichText::new("↺  Recargar biblioteca").font(FontId::proportional(12.0)).color(C_TEXT)).fill(Color32::from_rgb(28, 38, 55)).rounding(Rounding::same(5.0)).min_size(Vec2::new(160.0, 30.0))).clicked() { s.reload(); }
                                ui.add_space(8.0);
                                if ui.add(egui::Button::new(RichText::new("Abrir Steam").font(FontId::proportional(12.0)).color(C_TEXT)).fill(Color32::from_rgb(28, 38, 55)).rounding(Rounding::same(5.0)).min_size(Vec2::new(110.0, 30.0))).clicked() {
                                    Command::new("cmd").args(["/C", "start", "", r"C:\Program Files (x86)\Steam\steam.exe"]).spawn().ok();
                                }
                            });
                            ui.add_space(12.0);
                            if let Some(cfg) = &s.config {
                                ui.label(RichText::new(format!("Steam: {}", cfg.steam_path)).font(FontId::proportional(11.0)).color(C_TEXT_FAINT));
                            }
                        });

                        ui.add_space(14.0);

                        // Zona de peligro
                        egui::Frame::none().fill(Color32::from_rgb(28, 18, 18)).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::same(18.0)).stroke(Stroke::new(1.0, Color32::from_rgb(80, 30, 30))).show(ui, |ui| {
                            ui.set_max_width(520.0);
                            ui.label(RichText::new("Zona de peligro").font(FontId::proportional(13.0)).color(C_RED).strong());
                            ui.add_space(10.0);
                            if ui.add(egui::Button::new(RichText::new("Resetear configuración").font(FontId::proportional(12.0)).color(C_RED)).fill(Color32::from_rgb(45, 15, 15)).rounding(Rounding::same(5.0)).min_size(Vec2::new(180.0, 30.0))).clicked() {
                                delete_config(); self.config = None; self.screen = Screen::Setup; *self.games.lock().unwrap() = vec![];
                            }
                        });
                    });
                });
                ui.add_space(20.0);
            });
        });
    }

    fn settings_section(&mut self, ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui, &mut SteamLite)) {
        egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::same(18.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
            ui.set_max_width(520.0);
            ui.label(RichText::new(title).font(FontId::proportional(13.0)).color(C_TEXT).strong());
            ui.add_space(12.0);
            content(ui, self);
        });
    }

    // ===================== LOGIN POPUP =====================

    fn show_login_popup(&mut self, ctx: &egui::Context) {
        let mut open = self.show_login;
        egui::Window::new("Iniciar sesión para descargar").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).open(&mut open).frame(egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(10.0)).stroke(Stroke::new(1.0, C_BORDER))).show(ctx, |ui| {
            ui.set_min_width(380.0);
            egui::Frame::none().inner_margin(egui::Margin::same(20.0)).show(ui, |ui| {
                ui.label(RichText::new("SteamCMD descargará el juego en segundo plano.").font(FontId::proportional(13.0)).color(C_TEXT));
                ui.add_space(4.0);
                ui.label(RichText::new("Tus credenciales se guardan solo en tu PC.").font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                ui.add_space(16.0);
                ui.label(RichText::new("Usuario").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                ui.add_space(3.0);
                ui.add(egui::TextEdit::singleline(&mut self.steamcmd_user).desired_width(340.0).text_color(C_TEXT));
                ui.add_space(10.0);
                ui.label(RichText::new("Contraseña").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                ui.add_space(3.0);
                ui.add(egui::TextEdit::singleline(&mut self.steamcmd_pass).desired_width(340.0).password(true).text_color(C_TEXT));
                ui.add_space(10.0);
                ui.label(RichText::new("Steam Guard (opcional)").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                ui.add_space(3.0);
                ui.add(egui::TextEdit::singleline(&mut self.steamcmd_guard).desired_width(200.0).hint_text("Déjalo vacío").text_color(C_TEXT));
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("⬇  Descargar").font(FontId::proportional(12.0)).color(Color32::WHITE).strong()).fill(C_BTN_GREEN).rounding(Rounding::same(5.0)).min_size(Vec2::new(130.0, 32.0))).clicked() {
                        if let Some(id) = self.pending_appid { self.show_login = false; self.do_download(id); self.pending_appid = None; }
                    }
                    ui.add_space(8.0);
                    if ui.add(egui::Button::new(RichText::new("Cancelar").font(FontId::proportional(12.0)).color(C_TEXT_DIM)).fill(Color32::from_rgb(28, 35, 48)).rounding(Rounding::same(5.0)).min_size(Vec2::new(90.0, 32.0))).clicked() {
                        self.show_login = false;
                        if let Some(id) = self.pending_appid { open::that(format!("steam://install/{}", id)).ok(); self.pending_appid = None; }
                    }
                });
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
        style.visuals.window_rounding = Rounding::same(10.0);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(28, 36, 50);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(38, 50, 70);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(45, 60, 85);
        style.visuals.widgets.inactive.rounding = Rounding::same(4.0);
        style.visuals.widgets.hovered.rounding = Rounding::same(4.0);
        style.visuals.selection.bg_fill = C_BTN;
        style.visuals.selection.stroke = Stroke::new(0.0, Color32::TRANSPARENT);
        style.spacing.item_spacing = Vec2::new(8.0, 4.0);
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
