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
const C_CARD:       Color32 = Color32::from_rgb(22, 27, 38);
const C_TOPBAR:     Color32 = Color32::from_rgb(13, 15, 21);
const C_BORDER:     Color32 = Color32::from_rgb(35, 45, 62);
const C_BORDER_LIT: Color32 = Color32::from_rgb(55, 80, 115);
const C_ACCENT:     Color32 = Color32::from_rgb(102, 192, 244);
const C_GREEN:      Color32 = Color32::from_rgb(74, 197, 90);
const C_TEXT:       Color32 = Color32::from_rgb(195, 208, 220);
const C_TEXT_DIM:   Color32 = Color32::from_rgb(110, 130, 155);
const C_TEXT_FAINT: Color32 = Color32::from_rgb(60, 75, 95);
const C_BTN:        Color32 = Color32::from_rgb(55, 125, 170);
const C_BTN_GREEN:  Color32 = Color32::from_rgb(50, 145, 60);
const C_RED:        Color32 = Color32::from_rgb(200, 70, 70);

// Card dimensions — compactas como Steam
const CW: f32 = 184.0;  // width
const CH: f32 = 87.0;   // image height (460x215 → ratio 0.467)
const CI: f32 = 58.0;   // info area height
const CT: f32 = CH + CI; // total card height

// ===================== CONFIG =====================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config { steam_path: String, steam_user: Option<String> }

fn config_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.pop(); p.push("steamlite_config.json"); p
}
fn names_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.pop(); p.push("steamlite_names.json"); p
}
fn load_config() -> Option<Config> { serde_json::from_str(&fs::read_to_string(config_path()).ok()?).ok() }
fn save_config(c: &Config) { if let Ok(j) = serde_json::to_string_pretty(c) { fs::write(config_path(), j).ok(); } }
fn delete_config() { fs::remove_file(config_path()).ok(); fs::remove_file(names_path()).ok(); }
fn load_names() -> HashMap<u64, String> { fs::read_to_string(names_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default() }
fn save_names(m: &HashMap<u64, String>) { if let Ok(j) = serde_json::to_string(m) { fs::write(names_path(), j).ok(); } }

// ===================== STRUCTS =====================

#[derive(Debug, Clone)]
struct Game { appid: u64, name: String, playtime_forever: u64, installed: bool }

impl Game {
    fn hours_str(&self) -> String {
        let h = self.playtime_forever as f32 / 60.0;
        if self.playtime_forever == 0 { "Sin jugar".into() }
        else if h < 1.0 { format!("{} min", self.playtime_forever) }
        else { format!("{:.1}h", h) }
    }
    fn img_url(&self) -> String { format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{}/header.jpg", self.appid) }
    fn launch(&self) -> String { format!("steam://run/{}", self.appid) }
    fn store(&self) -> String { format!("https://store.steampowered.com/app/{}", self.appid) }
    fn unnamed(&self) -> bool { self.name.starts_with("App ") }
}

#[derive(Debug, Clone)]
struct Friend { name: String, state: u8, game: Option<String> }
impl Friend {
    fn status(&self) -> &str { match self.state { 1 => "En línea", 2 => "Ocupado", 3 => "Ausente", _ => "Desconectado" } }
    fn color(&self) -> Color32 { match self.state { 1 => C_GREEN, 2 => Color32::from_rgb(200,80,80), 3 => Color32::from_rgb(200,160,40), _ => C_TEXT_FAINT } }
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

fn vdf(s: &str, k: &str) -> Option<String> {
    let search = format!("\"{}\"", k);
    let pos = s.find(&search)?;
    let after = s[pos + search.len()..].trim_start();
    if after.starts_with('"') { let i = &after[1..]; Some(i[..i.find('"')?].to_string()) } else { None }
}

fn load_games(path: &str, names: &HashMap<u64, String>) -> Result<Vec<Game>, String> {
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
            let id: u64 = match vdf(&c, "appid").and_then(|s| s.parse().ok()) { Some(i) => i, _ => continue };
            let name = vdf(&c, "name").unwrap_or_else(|| names.get(&id).cloned().unwrap_or_else(|| format!("App {}", id)));
            let pt = vdf(&c, "playtime_forever").and_then(|s| s.parse().ok()).unwrap_or(0u64);
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
                                let name = names.get(&id).cloned().unwrap_or_else(|| format!("App {}", id));
                                all.insert(id, Game { appid: id, name, playtime_forever: cur_pt, installed: false });
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

fn fetch_name(appid: u64) -> Option<String> {
    let url = format!("https://store.steampowered.com/api/appdetails?appids={}&filters=basic", appid);
    let resp = reqwest::blocking::Client::builder().user_agent("Mozilla/5.0").timeout(std::time::Duration::from_secs(8)).build().ok()?.get(&url).send().ok()?;
    let json: serde_json::Value = resp.json().ok()?;
    json[appid.to_string()]["data"]["name"].as_str().map(|s| s.to_string())
}

fn fetch_img(url: &str) -> Option<Vec<u8>> {
    let r = reqwest::blocking::Client::builder().user_agent("Mozilla/5.0").timeout(std::time::Duration::from_secs(10)).build().ok()?.get(url).send().ok()?;
    if r.status().is_success() { r.bytes().ok().map(|b| b.to_vec()) } else { None }
}

fn find_steamcmd(path: &str) -> Option<PathBuf> {
    for p in [PathBuf::from(path).join("steamcmd.exe"), PathBuf::from(r"C:\steamcmd\steamcmd.exe")] { if p.exists() { return Some(p); } }
    None
}

fn do_download(path: &str, user: &str, pass: &str, guard: &str, appid: u64, status: Arc<Mutex<String>>) {
    *status.lock().unwrap() = "Conectando...".into();
    let cmd = match find_steamcmd(path) { Some(p) => p, None => { open::that(format!("steam://install/{}", appid)).ok(); *status.lock().unwrap() = "Abriendo Steam...".into(); return; } };
    let dir = PathBuf::from(path).join("steamapps").join("common");
    let mut args = vec!["+login".into(), user.into(), pass.into()];
    if !guard.is_empty() { args.push(guard.into()); }
    args.extend(["+force_install_dir".into(), dir.to_string_lossy().into_owned(), "+app_update".into(), appid.to_string(), "validate".into(), "+quit".into()]);
    match Command::new(&cmd).args(&args).output() {
        Ok(out) => {
            let t = String::from_utf8_lossy(&out.stdout);
            if t.contains("Success") { *status.lock().unwrap() = "¡Descarga completada!".into(); }
            else if t.contains("guard") { *status.lock().unwrap() = "Necesita Steam Guard. Introducelo en Ajustes.".into(); }
            else if t.contains("Invalid") { *status.lock().unwrap() = "Contraseña incorrecta.".into(); }
            else { open::that(format!("steam://install/{}", appid)).ok(); *status.lock().unwrap() = "Abriendo Steam...".into(); }
        }
        Err(_) => { open::that(format!("steam://install/{}", appid)).ok(); *status.lock().unwrap() = "Abriendo Steam...".into(); }
    }
}

// ===================== APP =====================

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
    names: Arc<Mutex<HashMap<u64, String>>>,
    loading: Arc<Mutex<bool>>,
    resolving: Arc<Mutex<bool>>,
    load_error: Arc<Mutex<String>>,
    dl_status: Arc<Mutex<String>>,

    search: String,
    tab: Tab,
    filter_installed: bool,

    textures: HashMap<u64, egui::TextureHandle>,
    pending_imgs: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
    fetching: Arc<Mutex<HashSet<u64>>>,
    last_played: Option<String>,

    sc_user: String,
    sc_pass: String,
    sc_guard: String,
    show_login: bool,
    pending_id: Option<u64>,
}

impl App {
    fn new(_cc: &eframe::CreationContext) -> Self {
        let config = load_config();
        let screen = if config.is_some() { Screen::Main } else { Screen::Setup };
        let auto = find_steam().unwrap_or_default();
        let user = config.as_ref().and_then(|c| c.steam_user.clone()).unwrap_or_default();
        Self {
            screen, config, input_path: auto.clone(), setup_error: String::new(), autodetected: !auto.is_empty(),
            games: Arc::new(Mutex::new(vec![])), friends: Arc::new(Mutex::new(vec![])),
            names: Arc::new(Mutex::new(load_names())),
            loading: Arc::new(Mutex::new(false)), resolving: Arc::new(Mutex::new(false)),
            load_error: Arc::new(Mutex::new(String::new())), dl_status: Arc::new(Mutex::new(String::new())),
            search: String::new(), tab: Tab::Library, filter_installed: false,
            textures: HashMap::new(), pending_imgs: Arc::new(Mutex::new(vec![])),
            fetching: Arc::new(Mutex::new(HashSet::new())), last_played: None,
            sc_user: user, sc_pass: String::new(), sc_guard: String::new(), show_login: false, pending_id: None,
        }
    }

    fn reload(&self) {
        let cfg = match &self.config { Some(c) => c.clone(), None => return };
        let (g, f, l, e, n) = (Arc::clone(&self.games), Arc::clone(&self.friends), Arc::clone(&self.loading), Arc::clone(&self.load_error), Arc::clone(&self.names));
        *l.lock().unwrap() = true; *e.lock().unwrap() = String::new();
        thread::spawn(move || {
            let cache = n.lock().unwrap().clone();
            match load_games(&cfg.steam_path, &cache) { Ok(gs) => *g.lock().unwrap() = gs, Err(er) => *e.lock().unwrap() = er }
            *f.lock().unwrap() = load_friends(&cfg.steam_path);
            *l.lock().unwrap() = false;
        });
    }

    fn resolve_names(&self) {
        let g = Arc::clone(&self.games);
        let n = Arc::clone(&self.names);
        let r = Arc::clone(&self.resolving);
        if *r.lock().unwrap() { return; }
        *r.lock().unwrap() = true;
        thread::spawn(move || {
            let ids: Vec<u64> = {
                let gs = g.lock().unwrap();
                let cache = n.lock().unwrap();
                gs.iter().filter(|g| g.unnamed() && !cache.contains_key(&g.appid)).map(|g| g.appid).take(20).collect()
            };
            let mut updated = false;
            for id in ids {
                if let Some(name) = fetch_name(id) {
                    n.lock().unwrap().insert(id, name.clone());
                    if let Some(g) = g.lock().unwrap().iter_mut().find(|g| g.appid == id) { g.name = name; }
                    updated = true;
                }
                thread::sleep(std::time::Duration::from_millis(250));
            }
            if updated { save_names(&n.lock().unwrap()); }
            *r.lock().unwrap() = false;
        });
    }

    fn req_img(&self, id: u64, url: String) {
        let mut f = self.fetching.lock().unwrap();
        if f.contains(&id) { return; } f.insert(id); drop(f);
        let (p, fa) = (Arc::clone(&self.pending_imgs), Arc::clone(&self.fetching));
        thread::spawn(move || { if let Some(b) = fetch_img(&url) { p.lock().unwrap().push((id, b)); } fa.lock().unwrap().remove(&id); });
    }

    fn setup(&mut self) {
        let path = self.input_path.trim().to_string();
        if path.is_empty() { self.setup_error = "Introduce la ruta de Steam".into(); return; }
        if !PathBuf::from(&path).join("steamapps").exists() { self.setup_error = "No se encontró steamapps en esa ruta".into(); return; }
        let cfg = Config { steam_path: path, steam_user: None };
        save_config(&cfg); self.config = Some(cfg); self.setup_error = String::new(); self.screen = Screen::Main; self.reload();
    }

    fn start_download(&mut self, id: u64) {
        if self.sc_user.is_empty() || self.sc_pass.is_empty() { self.pending_id = Some(id); self.show_login = true; return; }
        let path = self.config.as_ref().map(|c| c.steam_path.clone()).unwrap_or_default();
        let (u, p, g) = (self.sc_user.clone(), self.sc_pass.clone(), self.sc_guard.clone());
        let status = Arc::clone(&self.dl_status);
        if let Some(cfg) = &mut self.config { cfg.steam_user = Some(u.clone()); save_config(cfg); }
        thread::spawn(move || { do_download(&path, &u, &p, &g, id, status); });
    }
}

// ===================== SETUP SCREEN =====================

impl App {
    fn show_setup(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            // Barra de acento top
            let r = ui.max_rect();
            ui.painter().rect_filled(Rect::from_min_max(r.min, Pos2::new(r.max.x, r.min.y + 2.0)), Rounding::ZERO, C_ACCENT);

            ui.add_space(100.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("STEAM LITE").font(FontId::proportional(40.0)).color(C_ACCENT).strong());
                ui.add_space(8.0);
                ui.label(RichText::new("Cliente ligero · Bajo consumo de RAM").font(FontId::proportional(13.0)).color(C_TEXT_DIM));
                ui.add_space(48.0);

                egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::same(30.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(RichText::new("Configuración inicial").font(FontId::proportional(16.0)).color(C_TEXT).strong());
                    ui.add_space(4.0);
                    ui.label(RichText::new("Solo necesitas hacerlo una vez").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                    ui.add_space(22.0);

                    if self.autodetected {
                        egui::Frame::none().fill(Color32::from_rgb(18, 45, 22)).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(12.0, 8.0)).stroke(Stroke::new(1.0, Color32::from_rgb(40, 100, 50))).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("✓").color(C_GREEN).font(FontId::proportional(12.0)));
                                ui.add_space(5.0);
                                ui.label(RichText::new("Steam detectado automáticamente").font(FontId::proportional(12.0)).color(C_GREEN));
                            });
                        });
                        ui.add_space(14.0);
                    }

                    ui.label(RichText::new("Ruta de instalación de Steam").font(FontId::proportional(12.0)).color(C_TEXT_DIM));
                    ui.add_space(5.0);
                    egui::Frame::none().fill(Color32::from_rgb(12, 15, 22)).rounding(Rounding::same(5.0)).stroke(Stroke::new(1.0, C_BORDER_LIT)).inner_margin(egui::Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.input_path).desired_width(360.0).hint_text(r"C:\Program Files (x86)\Steam").frame(false).text_color(C_TEXT));
                    });

                    if !self.setup_error.is_empty() {
                        ui.add_space(10.0);
                        egui::Frame::none().fill(Color32::from_rgb(45, 15, 15)).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
                            ui.label(RichText::new(&self.setup_error).color(C_RED).font(FontId::proportional(12.0)));
                        });
                    }

                    ui.add_space(22.0);
                    let btn = ui.add_sized(Vec2::new(360.0, 38.0), egui::Button::new(RichText::new("Entrar →").font(FontId::proportional(13.0)).color(Color32::WHITE).strong()).fill(C_BTN).rounding(Rounding::same(5.0)));
                    if btn.clicked() { self.setup(); }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) { self.setup(); }
                });

                ui.add_space(18.0);
                ui.label(RichText::new("🔒  Tus datos permanecen en tu PC. Sin telemetría.").font(FontId::proportional(11.0)).color(C_TEXT_FAINT));
            });
        });
    }
}

// ===================== MAIN =====================

impl App {
    fn show_main(&mut self, ctx: &egui::Context) {
        // Procesar texturas
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

        // Resolver nombres
        {
            let unnamed = self.games.lock().unwrap().iter().any(|g| g.unnamed());
            if unnamed && !*self.resolving.lock().unwrap() { self.resolve_names(); }
        }

        // SIDEBAR
        egui::SidePanel::left("sb").exact_width(200.0).frame(egui::Frame::none().fill(C_SIDEBAR)).show(ctx, |ui| {
            ui.set_min_height(ui.available_height());

            // Logo
            egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(16.0, 15.0)).show(ui, |ui| {
                ui.set_min_width(200.0);
                ui.label(RichText::new("STEAM LITE").font(FontId::proportional(15.0)).color(C_ACCENT).strong());
                ui.add_space(3.0);
                let cnt = self.games.lock().unwrap().len();
                let loading = *self.loading.lock().unwrap();
                let resolving = *self.resolving.lock().unwrap();
                let sub = if loading { "Cargando...".into() }
                    else if resolving { format!("{} juegos · Resolviendo nombres...", cnt) }
                    else { format!("{} juegos en biblioteca", cnt) };
                ui.label(RichText::new(sub).font(FontId::proportional(10.5)).color(C_TEXT_DIM));
            });

            ui.add_space(8.0);

            // Nav
            for (ic, lb, t) in [("🎮", "Biblioteca", Tab::Library), ("👥", "Amigos", Tab::Friends), ("⚙", "Ajustes", Tab::Settings)] {
                let sel = self.tab == t;
                let resp = egui::Frame::none()
                    .fill(if sel { Color32::from_rgb(20, 35, 55) } else { Color32::TRANSPARENT })
                    .inner_margin(egui::Margin::symmetric(16.0, 10.0))
                    .show(ui, |ui| {
                        ui.set_min_width(200.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(ic).font(FontId::proportional(12.0)).color(if sel { C_ACCENT } else { C_TEXT_DIM }));
                            ui.add_space(8.0);
                            ui.label(RichText::new(lb).font(FontId::proportional(12.5)).color(if sel { C_TEXT } else { C_TEXT_DIM }));
                        });
                    });
                if sel {
                    let r = resp.response.rect;
                    ui.painter().rect_filled(Rect::from_min_max(r.min, Pos2::new(r.min.x + 3.0, r.max.y)), Rounding::ZERO, C_ACCENT);
                }
                if resp.response.interact(egui::Sense::click()).clicked() { self.tab = t; }
            }

            ui.add_space(10.0);

            // Separador
            ui.add(egui::Separator::default().spacing(0.0));
            ui.add_space(10.0);

            // Estado
            let ds = self.dl_status.lock().unwrap().clone();
            egui::Frame::none().inner_margin(egui::Margin::symmetric(16.0, 0.0)).show(ui, |ui| {
                if !ds.is_empty() {
                    ui.label(RichText::new(&ds).font(FontId::proportional(10.5)).color(C_GREEN));
                } else if let Some(name) = &self.last_played.clone() {
                    ui.label(RichText::new("EN JUEGO").font(FontId::proportional(9.0)).color(C_TEXT_FAINT).strong());
                    ui.add_space(2.0);
                    ui.label(RichText::new(name).font(FontId::proportional(11.0)).color(C_GREEN));
                }
            });

            // Bottom
            let av = ui.available_height();
            if av > 32.0 { ui.add_space(av - 32.0); }
            egui::Frame::none().inner_margin(egui::Margin::symmetric(16.0, 5.0)).show(ui, |ui| {
                if ui.add(egui::Button::new(RichText::new("Cerrar sesión").font(FontId::proportional(10.5)).color(C_TEXT_FAINT)).fill(Color32::TRANSPARENT).rounding(Rounding::same(3.0))).clicked() {
                    delete_config(); self.config = None; self.screen = Screen::Setup; *self.games.lock().unwrap() = vec![];
                }
            });
        });

        // Error
        let err = self.load_error.lock().unwrap().clone();
        if !err.is_empty() {
            egui::TopBottomPanel::top("err").frame(egui::Frame::none().fill(Color32::from_rgb(80, 18, 18)).inner_margin(egui::Margin::symmetric(16.0, 8.0))).show(ctx, |ui| {
                ui.horizontal(|ui| { ui.label(RichText::new("⚠").color(C_RED)); ui.add_space(6.0); ui.label(RichText::new(&err).color(Color32::from_rgb(255, 180, 180)).font(FontId::proportional(12.0))); });
            });
        }

        if self.show_login { self.login_popup(ctx); }

        let tab = self.tab.clone();
        match tab {
            Tab::Library => self.show_library(ctx),
            Tab::Friends => self.show_friends(ctx),
            Tab::Settings => self.show_settings(ctx),
        }

        if !self.pending_imgs.lock().unwrap().is_empty() || *self.loading.lock().unwrap() || *self.resolving.lock().unwrap() {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        }
    }

    fn show_library(&mut self, ctx: &egui::Context) {
        // Topbar
        egui::TopBottomPanel::top("lt").frame(egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(16.0, 9.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Search
                egui::Frame::none().fill(Color32::from_rgb(12, 15, 22)).rounding(Rounding::same(4.0)).stroke(Stroke::new(1.0, C_BORDER_LIT)).inner_margin(egui::Margin::symmetric(9.0, 5.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("🔍").font(FontId::proportional(11.0)).color(C_TEXT_DIM));
                        ui.add_space(3.0);
                        ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(180.0).hint_text("Buscar...").frame(false).text_color(C_TEXT));
                    });
                });

                ui.add_space(8.0);

                // Filtros
                for (lb, active) in [("Todos", !self.filter_installed), ("Instalados", self.filter_installed)] {
                    let b = ui.add(egui::Button::new(RichText::new(lb).font(FontId::proportional(11.5)).color(if active { C_ACCENT } else { C_TEXT_DIM }))
                        .fill(if active { Color32::from_rgb(20, 38, 58) } else { Color32::TRANSPARENT })
                        .rounding(Rounding::same(4.0)));
                    if b.clicked() { self.filter_installed = lb == "Instalados"; }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("↺").font(FontId::proportional(13.0)).color(C_TEXT_DIM)).fill(Color32::TRANSPARENT).rounding(Rounding::same(4.0))).clicked() { self.reload(); }
                });
            });
        });

        let snap: Vec<Game> = {
            let gs = self.games.lock().unwrap();
            let q = self.search.to_lowercase();
            gs.iter().filter(|g| {
                (q.is_empty() || g.name.to_lowercase().contains(&q)) && (!self.filter_installed || g.installed)
            }).cloned().collect()
        };

        for g in snap.iter().take(50) { if !self.textures.contains_key(&g.appid) { self.req_img(g.appid, g.img_url()); } }

        let mut launch: Option<Game> = None;
        let mut install: Option<u64> = None;
        let mut open_store: Option<String> = None;

        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            if snap.is_empty() {
                ui.centered_and_justified(|ui| { ui.label(RichText::new("No hay juegos").font(FontId::proportional(15.0)).color(C_TEXT_DIM)); });
                return;
            }

            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(12.0);

                let sp = 8.0_f32;
                let pad = 24.0_f32;
                let avail = ui.available_width() - pad;
                let cols = ((avail + sp) / (CW + sp)).floor().max(1.0) as usize;
                let total_w = cols as f32 * CW + (cols - 1) as f32 * sp;
                let lm = ((avail - total_w) / 2.0).max(12.0);

                ui.horizontal(|ui| {
                    ui.add_space(lm);

                    egui::Grid::new("grid")
                        .num_columns(cols)
                        .spacing(Vec2::splat(sp))
                        .min_col_width(CW)
                        .max_col_width(CW)
                        .show(ui, |ui| {
                            for (i, g) in snap.iter().enumerate() {
                                if i > 0 && i % cols == 0 { ui.end_row(); }

                                // Card container con altura FIJA
                                let (card_rect, card_resp) = ui.allocate_exact_size(Vec2::new(CW, CT), egui::Sense::hover());

                                // Fondo
                                let hovered = card_resp.hovered();
                                let bg = if hovered { Color32::from_rgb(28, 35, 50) } else { C_CARD };
                                let border = if g.installed { C_BORDER_LIT } else { C_BORDER };
                                ui.painter().rect_filled(card_rect, Rounding::same(6.0), bg);
                                ui.painter().rect_stroke(card_rect, Rounding::same(6.0), Stroke::new(1.0, border));

                                // Imagen (siempre CH alto)
                                let img_rect = Rect::from_min_size(card_rect.min, Vec2::new(CW, CH));
                                if let Some(tex) = self.textures.get(&g.appid) {
                                    let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                                    ui.painter().image(tex.id(), img_rect, uv, Color32::WHITE);
                                    // Redondeo top
                                    let clip = ui.painter().clip_rect();
                                    ui.painter().with_clip_rect(clip).rect_filled(
                                        Rect::from_min_size(img_rect.min, Vec2::new(CW, 6.0)),
                                        Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 },
                                        Color32::TRANSPARENT,
                                    );
                                } else {
                                    // Placeholder con gradiente simulado
                                    ui.painter().rect_filled(img_rect, Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 }, Color32::from_rgb(18, 22, 32));
                                    // Spinner
                                    let t = ctx.input(|i| i.time) as f32;
                                    let c = img_rect.center();
                                    for k in 0..8 {
                                        let a = t * 3.0 + k as f32 * std::f32::consts::TAU / 8.0;
                                        let alpha = (k as f32 / 8.0 * 160.0) as u8 + 40;
                                        let off = Pos2::new(c.x + a.cos() * 9.0, c.y + a.sin() * 9.0);
                                        ui.painter().circle_filled(off, 2.0, Color32::from_rgba_unmultiplied(102, 192, 244, alpha));
                                    }
                                }

                                // Overlay gradiente bottom de imagen
                                let grad_rect = Rect::from_min_size(Pos2::new(img_rect.min.x, img_rect.max.y - 20.0), Vec2::new(CW, 20.0));
                                ui.painter().rect_filled(grad_rect, Rounding::ZERO, Color32::from_rgba_unmultiplied(22, 27, 38, 120));

                                // Info area
                                let info_rect = Rect::from_min_size(Pos2::new(card_rect.min.x, img_rect.max.y), Vec2::new(CW, CI));

                                // Nombre
                                let name_pos = Pos2::new(info_rect.min.x + 8.0, info_rect.min.y + 6.0);
                                let short_name = if g.name.len() > 22 { format!("{}...", &g.name[..19]) } else { g.name.clone() };
                                ui.painter().text(name_pos, egui::Align2::LEFT_TOP, &short_name, FontId::proportional(11.0), C_TEXT);

                                // Estado / horas
                                let sub_pos = Pos2::new(info_rect.min.x + 8.0, info_rect.min.y + 21.0);
                                if g.installed {
                                    let (dot_r, _) = (Rect::from_center_size(Pos2::new(sub_pos.x + 3.0, sub_pos.y + 4.0), Vec2::new(6.0, 6.0)), ());
                                    ui.painter().circle_filled(Pos2::new(sub_pos.x + 3.0, sub_pos.y + 4.0), 3.0, C_GREEN);
                                    ui.painter().text(Pos2::new(sub_pos.x + 10.0, sub_pos.y), egui::Align2::LEFT_TOP, "Instalado", FontId::proportional(9.5), C_GREEN);
                                } else {
                                    ui.painter().text(sub_pos, egui::Align2::LEFT_TOP, g.hours_str(), FontId::proportional(9.5), C_TEXT_DIM);
                                }

                                // Botones
                                let btn_y = info_rect.min.y + 34.0;
                                let play_rect = Rect::from_min_size(Pos2::new(info_rect.min.x + 7.0, btn_y), Vec2::new(88.0, 20.0));
                                let more_rect = Rect::from_min_size(Pos2::new(info_rect.min.x + 99.0, btn_y), Vec2::new(22.0, 20.0));

                                let play_hov = ctx.input(|i| i.pointer.hover_pos().map(|p| play_rect.contains(p)).unwrap_or(false));
                                let more_hov = ctx.input(|i| i.pointer.hover_pos().map(|p| more_rect.contains(p)).unwrap_or(false));

                                let (play_col, play_text) = if g.installed {
                                    (if play_hov { Color32::from_rgb(70, 150, 200) } else { C_BTN }, "▶  Jugar")
                                } else {
                                    (if play_hov { Color32::from_rgb(65, 170, 75) } else { C_BTN_GREEN }, "⬇  Instalar")
                                };

                                ui.painter().rect_filled(play_rect, Rounding::same(3.0), play_col);
                                ui.painter().text(play_rect.center(), egui::Align2::CENTER_CENTER, play_text, FontId::proportional(9.5), Color32::WHITE);

                                let more_col = if more_hov { Color32::from_rgb(40, 52, 72) } else { Color32::from_rgb(28, 36, 52) };
                                ui.painter().rect_filled(more_rect, Rounding::same(3.0), more_col);
                                ui.painter().text(more_rect.center(), egui::Align2::CENTER_CENTER, "···", FontId::proportional(11.0), C_TEXT_DIM);

                                // Click handling
                                let pointer = ctx.input(|i| i.pointer.clone());
                                if pointer.any_pressed() {
                                    if let Some(pos) = pointer.press_origin() {
                                        if play_rect.contains(pos) {
                                            if g.installed { launch = Some(g.clone()); }
                                            else { install = Some(g.appid); }
                                        }
                                        if more_rect.contains(pos) { open_store = Some(g.store()); }
                                    }
                                }
                            }
                        });
                });

                ui.add_space(20.0);
            });
        });

        if let Some(g) = launch { open::that(g.launch()).ok(); self.last_played = Some(g.name); ctx.request_repaint(); }
        if let Some(id) = install { self.start_download(id); }
        if let Some(url) = open_store { open::that(url).ok(); }
    }

    fn show_friends(&mut self, ctx: &egui::Context) {
        let friends = self.friends.lock().unwrap().clone();

        egui::TopBottomPanel::top("ft").frame(egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(16.0, 11.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Amigos").font(FontId::proportional(14.0)).color(C_TEXT).strong());
                ui.add_space(10.0);
                let on = friends.iter().filter(|f| f.state > 0).count();
                ui.label(RichText::new(format!("{} en línea  ·  {} total", on, friends.len())).font(FontId::proportional(11.5)).color(C_TEXT_DIM));
            });
        });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            if friends.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(RichText::new("👥").font(FontId::proportional(36.0)));
                        ui.add_space(10.0);
                        ui.label(RichText::new("Lista de amigos vacía").font(FontId::proportional(14.0)).color(C_TEXT_DIM));
                        ui.add_space(5.0);
                        ui.label(RichText::new("Abre Steam una vez para sincronizar.").font(FontId::proportional(11.5)).color(C_TEXT_FAINT));
                    });
                });
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(10.0);
                let online: Vec<&Friend> = friends.iter().filter(|f| f.state > 0).collect();
                let offline: Vec<&Friend> = friends.iter().filter(|f| f.state == 0).collect();

                if !online.is_empty() {
                    ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new(format!("EN LÍNEA — {}", online.len())).font(FontId::proportional(9.5)).color(C_TEXT_FAINT).strong()); });
                    ui.add_space(5.0);
                    for f in &online { self.friend_row(ui, f); }
                    ui.add_space(12.0);
                }
                if !offline.is_empty() {
                    ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new(format!("DESCONECTADO — {}", offline.len())).font(FontId::proportional(9.5)).color(C_TEXT_FAINT).strong()); });
                    ui.add_space(5.0);
                    for f in &offline { self.friend_row(ui, f); }
                }
                ui.add_space(14.0);
            });
        });
    }

    fn friend_row(&self, ui: &mut egui::Ui, f: &Friend) {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            egui::Frame::none().fill(C_CARD).rounding(Rounding::same(5.0)).inner_margin(egui::Margin::symmetric(11.0, 8.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
                ui.set_min_width(ui.available_width() - 14.0);
                ui.horizontal(|ui| {
                    // Avatar
                    let (r, _) = ui.allocate_exact_size(Vec2::new(30.0, 30.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, Rounding::same(4.0), Color32::from_rgb(26, 35, 52));
                    let init = f.name.chars().next().unwrap_or('?').to_uppercase().to_string();
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, &init, FontId::proportional(13.0), C_TEXT_DIM);
                    // Punto estado
                    let dot = Pos2::new(r.max.x - 3.0, r.max.y - 3.0);
                    ui.painter().circle_filled(dot, 5.0, C_BG);
                    ui.painter().circle_filled(dot, 4.0, f.color());

                    ui.add_space(9.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&f.name).font(FontId::proportional(12.5)).color(C_TEXT).strong());
                        if let Some(g) = &f.game {
                            ui.label(RichText::new(format!("▶  {}", g)).font(FontId::proportional(10.5)).color(C_GREEN));
                        } else {
                            ui.label(RichText::new(f.status()).font(FontId::proportional(10.5)).color(C_TEXT_DIM));
                        }
                    });
                });
            });
        });
        ui.add_space(3.0);
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("st").frame(egui::Frame::none().fill(C_TOPBAR).inner_margin(egui::Margin::symmetric(16.0, 11.0))).show(ctx, |ui| {
            ui.label(RichText::new("Ajustes").font(FontId::proportional(14.0)).color(C_TEXT).strong());
        });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(C_BG)).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_max_width(500.0);

                        // SteamCMD
                        egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(7.0)).inner_margin(egui::Margin::same(16.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
                            ui.set_max_width(500.0);
                            ui.label(RichText::new("SteamCMD — Descarga directa de juegos").font(FontId::proportional(12.5)).color(C_TEXT).strong());
                            ui.add_space(10.0);
                            ui.label(RichText::new("Usuario").font(FontId::proportional(11.5)).color(C_TEXT_DIM));
                            ui.add_space(3.0);
                            ui.add(egui::TextEdit::singleline(&mut self.sc_user).desired_width(280.0).text_color(C_TEXT));
                            ui.add_space(8.0);
                            ui.label(RichText::new("Contraseña").font(FontId::proportional(11.5)).color(C_TEXT_DIM));
                            ui.add_space(3.0);
                            ui.add(egui::TextEdit::singleline(&mut self.sc_pass).desired_width(280.0).password(true).text_color(C_TEXT));
                            ui.add_space(8.0);
                            ui.label(RichText::new("Steam Guard (opcional)").font(FontId::proportional(11.5)).color(C_TEXT_DIM));
                            ui.add_space(3.0);
                            ui.add(egui::TextEdit::singleline(&mut self.sc_guard).desired_width(200.0).hint_text("Déjalo vacío").text_color(C_TEXT));
                        });

                        ui.add_space(10.0);

                        // Acciones
                        egui::Frame::none().fill(C_PANEL).rounding(Rounding::same(7.0)).inner_margin(egui::Margin::same(16.0)).stroke(Stroke::new(1.0, C_BORDER)).show(ui, |ui| {
                            ui.set_max_width(500.0);
                            ui.label(RichText::new("Acciones").font(FontId::proportional(12.5)).color(C_TEXT).strong());
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new(RichText::new("↺  Recargar").font(FontId::proportional(12.0)).color(C_TEXT)).fill(Color32::from_rgb(26, 36, 52)).rounding(Rounding::same(4.0)).min_size(Vec2::new(110.0, 28.0))).clicked() { self.reload(); }
                                ui.add_space(6.0);
                                let steam_exe = self.config.as_ref().map(|c| PathBuf::from(&c.steam_path).join("steam.exe")).unwrap_or_default();
                                if ui.add(egui::Button::new(RichText::new("Abrir Steam mínimo").font(FontId::proportional(12.0)).color(C_TEXT)).fill(Color32::from_rgb(26, 36, 52)).rounding(Rounding::same(4.0)).min_size(Vec2::new(140.0, 28.0))).clicked() {
                                    Command::new(&steam_exe).args(["-no-browser", "-silent"]).spawn().ok();
                                }
                            });
                            ui.add_space(8.0);
                            if let Some(cfg) = &self.config {
                                ui.label(RichText::new(format!("Steam: {}", cfg.steam_path)).font(FontId::proportional(10.5)).color(C_TEXT_FAINT));
                            }
                        });

                        ui.add_space(10.0);

                        egui::Frame::none().fill(Color32::from_rgb(26, 15, 15)).rounding(Rounding::same(7.0)).inner_margin(egui::Margin::same(16.0)).stroke(Stroke::new(1.0, Color32::from_rgb(65, 22, 22))).show(ui, |ui| {
                            ui.set_max_width(500.0);
                            ui.label(RichText::new("Zona de peligro").font(FontId::proportional(12.5)).color(C_RED).strong());
                            ui.add_space(8.0);
                            if ui.add(egui::Button::new(RichText::new("Resetear configuración").font(FontId::proportional(11.5)).color(C_RED)).fill(Color32::from_rgb(38, 10, 10)).rounding(Rounding::same(4.0)).min_size(Vec2::new(170.0, 28.0))).clicked() {
                                delete_config(); self.config = None; self.screen = Screen::Setup; *self.games.lock().unwrap() = vec![];
                            }
                        });
                    });
                });
            });
        });
    }

    fn login_popup(&mut self, ctx: &egui::Context) {
        let mut open = self.show_login;
        egui::Window::new("Iniciar sesión para descargar").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).open(&mut open).show(ctx, |ui| {
            egui::Frame::none().inner_margin(egui::Margin::same(18.0)).show(ui, |ui| {
                ui.set_min_width(340.0);
                ui.label(RichText::new("SteamCMD descargará el juego en segundo plano.").font(FontId::proportional(12.5)).color(C_TEXT));
                ui.add_space(14.0);
                ui.label(RichText::new("Usuario").font(FontId::proportional(11.5)).color(C_TEXT_DIM));
                ui.add_space(3.0);
                ui.add(egui::TextEdit::singleline(&mut self.sc_user).desired_width(300.0).text_color(C_TEXT));
                ui.add_space(8.0);
                ui.label(RichText::new("Contraseña").font(FontId::proportional(11.5)).color(C_TEXT_DIM));
                ui.add_space(3.0);
                ui.add(egui::TextEdit::singleline(&mut self.sc_pass).desired_width(300.0).password(true).text_color(C_TEXT));
                ui.add_space(8.0);
                ui.label(RichText::new("Steam Guard (opcional)").font(FontId::proportional(11.5)).color(C_TEXT_DIM));
                ui.add_space(3.0);
                ui.add(egui::TextEdit::singleline(&mut self.sc_guard).desired_width(180.0).hint_text("Déjalo vacío").text_color(C_TEXT));
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("⬇  Descargar").font(FontId::proportional(12.0)).color(Color32::WHITE).strong()).fill(C_BTN_GREEN).rounding(Rounding::same(4.0)).min_size(Vec2::new(120.0, 30.0))).clicked() {
                        if let Some(id) = self.pending_id { self.show_login = false; self.start_download(id); self.pending_id = None; }
                    }
                    ui.add_space(6.0);
                    if ui.add(egui::Button::new(RichText::new("Cancelar").font(FontId::proportional(12.0)).color(C_TEXT_DIM)).fill(Color32::from_rgb(26, 34, 48)).rounding(Rounding::same(4.0)).min_size(Vec2::new(85.0, 30.0))).clicked() {
                        self.show_login = false;
                        if let Some(id) = self.pending_id { open::that(format!("steam://install/{}", id)).ok(); self.pending_id = None; }
                    }
                });
            });
        });
        if !open { self.show_login = false; }
    }
}

// ===================== APP LOOP =====================

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut style = (*ctx.style()).clone();
        style.visuals.dark_mode = true;
        style.visuals.panel_fill = C_BG;
        style.visuals.window_fill = C_PANEL;
        style.visuals.window_rounding = Rounding::same(8.0);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(24, 32, 46);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(32, 44, 64);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(40, 55, 80);
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
            .with_inner_size([1200.0, 750.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    }, Box::new(|cc| Box::new(App::new(cc))))
}
