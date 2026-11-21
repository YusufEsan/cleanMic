#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chain;
mod diag;
mod dsp;
mod engine;
#[cfg(windows)]
mod exclusive;
mod settings;
mod sound;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ringbuf::traits::{Consumer, Observer};
use ringbuf::HeapCons;
use serde_json::{json, Value};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use chain::SbCmd;
use dsp::lin_to_db;
use engine::{fload, ftake, Engine, EngineInfo, MonCmd, Shared};
use settings::{apply_preset, DspParams, PadInfo, Settings, PAD_COUNT, PRESET_NAMES};

struct App {
    shared: Arc<Shared>,
    settings: Mutex<Settings>,
    engine: Mutex<Option<Engine>>,
    info: Mutex<Option<EngineInfo>>,
    error: Mutex<Option<String>>,
    sb_tx: Mutex<Option<Sender<SbCmd>>>,
    mon_tx: Mutex<Option<Sender<MonCmd>>>,
    rec: Mutex<Option<HeapCons<f32>>>,
    spec: Mutex<Option<(HeapCons<f32>, HeapCons<f32>)>>,
    pads: Mutex<Vec<Option<Arc<Vec<f32>>>>>,
    test: Mutex<Option<(Arc<Vec<f32>>, Arc<Vec<f32>>)>>,
    busy: AtomicBool,
    dirty: AtomicBool,
    /// Pencere görünürken true; gizliyken metre/dalga formu gönderilmez (CPU tasarrufu)
    ui_active: AtomicBool,
    path: PathBuf,
}

impl App {
    fn commit(&self, s: &Settings) {
        *self.shared.params.lock().unwrap() = s.dsp;
        self.shared.target_samples.store((s.dsp.buffer_ms.clamp(5.0, 100.0) * 48.0) as u32, Relaxed);
        self.shared.version.fetch_add(1, Relaxed);
        self.dirty.store(true, Relaxed);
    }

    fn modify_dsp(&self, f: impl FnOnce(&mut DspParams)) -> DspParams {
        let mut s = self.settings.lock().unwrap();
        f(&mut s.dsp);
        self.commit(&s);
        s.dsp
    }

    fn restart(&self) {
        *self.engine.lock().unwrap() = None; // eski stream'leri önce kapat
        let s = self.settings.lock().unwrap().clone();
        match engine::start(&s, self.shared.clone()) {
            Ok((eng, io)) => {
                *self.info.lock().unwrap() = Some(io.info);
                *self.error.lock().unwrap() = None;
                *self.sb_tx.lock().unwrap() = Some(io.sb_tx);
                *self.mon_tx.lock().unwrap() = Some(io.mon_tx);
                *self.rec.lock().unwrap() = Some(io.rec);
                *self.spec.lock().unwrap() = Some((io.spec_raw, io.spec_out));
                *self.engine.lock().unwrap() = Some(eng);
            }
            Err(e) => {
                *self.info.lock().unwrap() = None;
                *self.error.lock().unwrap() = Some(e);
                *self.sb_tx.lock().unwrap() = None;
                *self.mon_tx.lock().unwrap() = None;
                *self.rec.lock().unwrap() = None;
                *self.spec.lock().unwrap() = None;
            }
        }
    }

    fn status(&self) -> Value {
        json!({
            "running": self.engine.lock().unwrap().is_some(),
            "error": *self.error.lock().unwrap(),
            "info": *self.info.lock().unwrap(),
        })
    }

    fn save(&self) {
        let s = self.settings.lock().unwrap().clone();
        if let Ok(txt) = serde_json::to_string_pretty(&s) {
            let _ = std::fs::write(&self.path, txt);
        }
    }

    fn play_pad(&self, i: usize) {
        let buf = self.pads.lock().unwrap().get(i).cloned().flatten();
        if let (Some(b), Some(tx)) = (buf, self.sb_tx.lock().unwrap().as_ref()) {
            let _ = tx.send(SbCmd::Play(i, b));
        }
    }

    fn stop_pads(&self) {
        if let Some(tx) = self.sb_tx.lock().unwrap().as_ref() {
            let _ = tx.send(SbCmd::StopAll);
        }
    }

    fn monitor(&self, cmd: MonCmd) {
        if let Some(tx) = self.mon_tx.lock().unwrap().as_ref() {
            let _ = tx.send(cmd);
        }
    }

    /// Ham mikrofon sesini `secs` saniye kaydeder.
    fn record(&self, secs: f32) -> Result<Vec<f32>, String> {
        let mut g = self.rec.lock().unwrap();
        let cons = g.as_mut().ok_or("Ses motoru çalışmıyor")?;
        let stale = cons.occupied_len();
        cons.skip(stale);
        self.shared.recording.store(true, Relaxed);
        let need = (secs * 48_000.0) as usize;
        let mut out = Vec::with_capacity(need + 4_800);
        let deadline = Instant::now() + Duration::from_secs_f32(secs + 2.0);
        while out.len() < need && Instant::now() < deadline {
            while let Some(v) = cons.try_pop() {
                out.push(v);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.shared.recording.store(false, Relaxed);
        out.truncate(need);
        if out.len() < need / 2 {
            return Err("Mikrofondan ses gelmiyor".into());
        }
        Ok(out)
    }
}

type St<'a> = State<'a, Arc<App>>;

// ---------------------------------------------------------------- komutlar

#[tauri::command]
fn get_state(app: St, h: AppHandle) -> Value {
    let s = app.settings.lock().unwrap().clone();
    json!({
        "settings": s,
        "status": app.status(),
        "presets": PRESET_NAMES,
        "autostart": h.autolaunch().is_enabled().unwrap_or(false),
        "has_test": app.test.lock().unwrap().is_some(),
    })
}

/// Cihaz taraması yavaş olabilir (WASAPI); async olduğu için ana thread'i/pencereyi dondurmaz.
#[tauri::command]
async fn list_devices() -> Result<engine::DeviceList, String> {
    tauri::async_runtime::spawn_blocking(engine::list_devices).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn set_dsp(app: St, patch: Value) -> Result<DspParams, String> {
    let mut s = app.settings.lock().unwrap();
    let mut v = serde_json::to_value(s.dsp).map_err(|e| e.to_string())?;
    if let (Some(obj), Some(p)) = (v.as_object_mut(), patch.as_object()) {
        for (k, val) in p {
            if obj.contains_key(k) {
                obj.insert(k.clone(), val.clone());
            }
        }
    }
    s.dsp = serde_json::from_value(v).map_err(|e| e.to_string())?;
    app.commit(&s);
    Ok(s.dsp)
}

#[tauri::command]
fn set_preset(app: St, index: u8) -> DspParams {
    app.modify_dsp(|d| apply_preset(d, index))
}

const CUSTOM_MAX: usize = 12;

/// O anki pedal ayarlarını isimle kaydeder (aynı isim varsa üzerine yazar).
#[tauri::command]
fn save_custom(app: St, name: String) -> Result<Value, String> {
    let name: String = name.trim().chars().take(18).collect();
    if name.is_empty() {
        return Err("İsim boş olamaz".into());
    }
    let mut s = app.settings.lock().unwrap();
    let dsp = s.dsp;
    let idx = match s.custom_presets.iter().position(|c| c.name == name) {
        Some(i) => {
            s.custom_presets[i].dsp = dsp;
            i
        }
        None => {
            if s.custom_presets.len() >= CUSTOM_MAX {
                return Err(format!("En fazla {CUSTOM_MAX} mod kaydedilebilir; önce birini sil."));
            }
            s.custom_presets.push(settings::CustomPreset { name, dsp });
            s.custom_presets.len() - 1
        }
    };
    s.dsp.preset = settings::CUSTOM_BASE + idx as u8;
    s.dsp.preset_space = false;
    s.dsp.preset_studio = false;
    app.commit(&s);
    Ok(json!({ "custom": s.custom_presets, "dsp": s.dsp }))
}

#[tauri::command]
fn apply_custom(app: St, index: usize) -> Result<DspParams, String> {
    let mut s = app.settings.lock().unwrap();
    let src = s.custom_presets.get(index).ok_or("Mod bulunamadı")?.dsp;
    settings::apply_sound(&mut s.dsp, &src);
    s.dsp.preset = settings::CUSTOM_BASE + index as u8;
    app.commit(&s);
    Ok(s.dsp)
}

#[tauri::command]
fn rename_custom(app: St, index: usize, name: String) -> Result<Value, String> {
    let name: String = name.trim().chars().take(18).collect();
    if name.is_empty() {
        return Err("İsim boş olamaz".into());
    }
    let mut s = app.settings.lock().unwrap();
    if s.custom_presets.iter().enumerate().any(|(i, c)| i != index && c.name == name) {
        return Err("Bu isimde bir mod zaten var".into());
    }
    s.custom_presets.get_mut(index).ok_or("Mod bulunamadı")?.name = name;
    app.dirty.store(true, Relaxed);
    Ok(json!({ "custom": s.custom_presets, "dsp": s.dsp }))
}

/// Kayıtlı modu şu anki pedal ayarlarıyla günceller.
#[tauri::command]
fn update_custom(app: St, index: usize) -> Result<Value, String> {
    let mut s = app.settings.lock().unwrap();
    let dsp = s.dsp;
    s.custom_presets.get_mut(index).ok_or("Mod bulunamadı")?.dsp = dsp;
    s.dsp.preset = settings::CUSTOM_BASE + index as u8;
    s.dsp.preset_space = false;
    s.dsp.preset_studio = false;
    app.commit(&s);
    Ok(json!({ "custom": s.custom_presets, "dsp": s.dsp }))
}

#[tauri::command]
fn delete_custom(app: St, index: usize) -> Value {
    let mut s = app.settings.lock().unwrap();
    if index < s.custom_presets.len() {
        s.custom_presets.remove(index);
        let cur = settings::CUSTOM_BASE + index as u8;
        if s.dsp.preset == cur {
            s.dsp.preset = 255; // ses aynı kalır, sadece seçim kalkar
        } else if s.dsp.preset > cur && s.dsp.preset < 255 {
            s.dsp.preset -= 1;
        }
        app.commit(&s);
    }
    json!({ "custom": s.custom_presets, "dsp": s.dsp })
}

#[tauri::command]
fn set_devices(app: St, input: Option<String>, output: Option<String>, monitor: Option<String>) -> Value {
    {
        let mut s = app.settings.lock().unwrap();
        s.input_device = input;
        s.output_device = output;
        s.monitor_device = monitor;
    }
    app.dirty.store(true, Relaxed);
    app.restart();
    app.status()
}

/// CABLE çıkışını özel modda aç/kapat (ekran paylaşımında gizlenme) ve motoru yeniden başlat.
#[tauri::command]
async fn set_hide_capture(app: St<'_>, enabled: bool) -> Result<Value, String> {
    app.settings.lock().unwrap().hide_from_capture = enabled;
    app.dirty.store(true, Relaxed);
    let a = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        a.restart();
        a.status()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn restart_engine(app: St) -> Value {
    app.restart();
    app.status()
}

#[tauri::command]
async fn assign_pad(app: St<'_>, index: usize, path: String) -> Result<PadInfo, String> {
    if index >= PAD_COUNT {
        return Err("Geçersiz pad".into());
    }
    let p2 = path.clone();
    let buf = tauri::async_runtime::spawn_blocking(move || sound::load(&p2)).await.map_err(|e| e.to_string())??;
    let name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Ses")
        .chars()
        .take(14)
        .collect::<String>();
    let info = PadInfo { path, name };
    app.pads.lock().unwrap()[index] = Some(Arc::new(buf));
    app.settings.lock().unwrap().pads[index] = Some(info.clone());
    app.dirty.store(true, Relaxed);
    Ok(info)
}

#[tauri::command]
fn clear_pad(app: St, index: usize) {
    if index < PAD_COUNT {
        app.pads.lock().unwrap()[index] = None;
        app.settings.lock().unwrap().pads[index] = None;
        app.dirty.store(true, Relaxed);
    }
}

#[tauri::command]
fn play_pad(app: St, index: usize) {
    app.play_pad(index);
}

#[tauri::command]
fn stop_pads(app: St) {
    app.stop_pads();
}

#[tauri::command]
fn diag_run(app: St, h: AppHandle) -> Result<(), String> {
    if app.busy.swap(true, Relaxed) {
        return Err("Başka bir kayıt sürüyor".into());
    }
    let app = app.inner().clone();
    std::thread::spawn(move || {
        let run = || -> Result<diag::Report, String> {
            let u0 = app.shared.underruns.load(Relaxed);
            ftake(&app.shared.cpu);
            let _ = h.emit("diag", json!({"phase": "silence", "seconds": 3}));
            let silence = app.record(3.0)?;
            let _ = h.emit("diag", json!({"phase": "speech", "seconds": 5}));
            let speech = app.record(5.0)?;
            let info = app.info.lock().unwrap().clone().unwrap_or_default();
            let p = app.settings.lock().unwrap().dsp;
            let cpu = fload(&app.shared.cpu);
            Ok(diag::analyze(&silence, &speech, &info, app.shared.underruns.load(Relaxed) - u0, cpu, &p))
        };
        match run() {
            Ok(r) => {
                let _ = h.emit("diag", json!({"phase": "done", "report": r}));
            }
            Err(e) => {
                let _ = h.emit("diag", json!({"phase": "error", "error": e}));
            }
        }
        app.busy.store(false, Relaxed);
    });
    Ok(())
}

#[tauri::command]
fn test_record(app: St, h: AppHandle, seconds: f32) -> Result<(), String> {
    if app.busy.swap(true, Relaxed) {
        return Err("Başka bir kayıt sürüyor".into());
    }
    let app = app.inner().clone();
    let secs = seconds.clamp(3.0, 15.0);
    std::thread::spawn(move || {
        let _ = h.emit("test", json!({"phase": "recording", "seconds": secs}));
        match app.record(secs) {
            Ok(raw) => {
                let _ = h.emit("test", json!({"phase": "processing"}));
                let p = app.settings.lock().unwrap().dsp;
                let clean = engine::process_offline(&raw, &p);
                *app.test.lock().unwrap() = Some((Arc::new(raw), Arc::new(clean)));
                let has_monitor = app.info.lock().unwrap().as_ref().map(|i| i.monitor.is_some()).unwrap_or(false);
                let _ = h.emit("test", json!({"phase": "ready", "monitor": has_monitor}));
            }
            Err(e) => {
                let _ = h.emit("test", json!({"phase": "error", "error": e}));
            }
        }
        app.busy.store(false, Relaxed);
    });
    Ok(())
}

#[tauri::command]
fn test_play(app: St, which: String) -> Result<(), String> {
    let t = app.test.lock().unwrap().clone().ok_or("Önce kayıt yap")?;
    let buf = if which == "raw" { t.0 } else { t.1 };
    app.monitor(MonCmd::Play(buf));
    Ok(())
}

#[tauri::command]
fn test_stop(app: St) {
    app.monitor(MonCmd::Stop);
}

#[tauri::command]
fn set_ui_active(app: St, active: bool) {
    app.ui_active.store(active, Relaxed);
}

#[tauri::command]
fn set_autostart(h: AppHandle, enabled: bool) -> Result<bool, String> {
    let m = h.autolaunch();
    if enabled { m.enable() } else { m.disable() }.map_err(|e| e.to_string())?;
    m.is_enabled().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- kısayollar ve tepsi

#[derive(Clone, Copy)]
enum Act {
    Mute,
    Fx,
    Monitor,
    Preset(u8),
    Pad(usize),
    StopPads,
}

fn act(h: &AppHandle, a: Act) {
    let app = h.state::<Arc<App>>();
    let d = match a {
        Act::Mute => Some(app.modify_dsp(|d| d.mute = !d.mute)),
        Act::Fx => Some(app.modify_dsp(|d| d.fx_on = !d.fx_on)),
        Act::Monitor => Some(app.modify_dsp(|d| d.monitor_on = !d.monitor_on)),
        Act::Preset(i) => Some(app.modify_dsp(|d| apply_preset(d, i))),
        Act::Pad(i) => {
            app.play_pad(i);
            let _ = h.emit("pad", i);
            None
        }
        Act::StopPads => {
            app.stop_pads();
            None
        }
    };
    if let Some(d) = d {
        let _ = h.emit("dsp", d);
    }
}

fn show_main(h: &AppHandle) {
    h.state::<Arc<App>>().ui_active.store(true, Relaxed);
    if let Some(w) = h.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn setup_hotkeys(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let m = Some(Modifiers::CONTROL | Modifiers::ALT);
    let ms = Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT);
    let fkeys = [
        Code::F1, Code::F2, Code::F3, Code::F4, Code::F5, Code::F6,
        Code::F7, Code::F8, Code::F9, Code::F10, Code::F11, Code::F12,
    ];
    let digits = [Code::Digit1, Code::Digit2, Code::Digit3, Code::Digit4, Code::Digit5, Code::Digit6, Code::Digit7, Code::Digit8];
    let mut binds: Vec<(Shortcut, Act)> = vec![
        (Shortcut::new(m, Code::KeyM), Act::Mute),
        (Shortcut::new(m, Code::KeyB), Act::Fx),
        (Shortcut::new(m, Code::KeyL), Act::Monitor),
        (Shortcut::new(m, Code::Digit0), Act::StopPads),
    ];
    // modlar 1–12: Ctrl+Alt+F1…F12, modlar 13–16: Ctrl+Alt+Shift+F1…F4
    for (i, c) in fkeys.iter().enumerate() {
        binds.push((Shortcut::new(m, *c), Act::Preset(i as u8)));
    }
    for (i, c) in fkeys.iter().take(settings::PRESET_COUNT - 12).enumerate() {
        binds.push((Shortcut::new(ms, *c), Act::Preset(12 + i as u8)));
    }
    for (i, c) in digits.iter().enumerate() {
        binds.push((Shortcut::new(m, *c), Act::Pad(i)));
    }
    let b2 = binds.clone();
    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |h, sc, ev| {
                if ev.state() == ShortcutState::Pressed {
                    if let Some((_, a)) = b2.iter().find(|(s, _)| s == sc) {
                        act(h, *a);
                    }
                }
            })
            .build(),
    )?;
    for (s, _) in &binds {
        if let Err(e) = app.global_shortcut().register(s.clone()) {
            eprintln!("kısayol kaydedilemedi: {e}");
        }
    }
    Ok(())
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "CleanMic'i göster", true, None::<&str>)?;
    let mute = MenuItem::with_id(app, "mute", "Sustur / aç   Ctrl+Alt+M", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &mute, &quit])?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().ok_or("ikon yok")?)
        .tooltip("CleanMic")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|h, ev| match ev.id().as_ref() {
            "show" => show_main(h),
            "mute" => act(h, Act::Mute),
            "quit" => {
                h.state::<Arc<App>>().save();
                h.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

// ---------------------------------------------------------------- arka plan thread'leri

fn meter_loop(h: AppHandle, app: Arc<App>) {
    let mut scope = diag::Scope::new();
    let mut retry_at: Option<Instant> = None;
    let mut tick = 0u64;
    loop {
        std::thread::sleep(Duration::from_millis(40)); // 25 kare/sn yeterince akıcı
        tick += 1;
        let sh = &app.shared;

        // cihaz koptuysa (USB çıkarıldı vb.) birkaç saniyede bir yeniden dene
        // Önemli: hatayı önce yerel değişkene al. `if let` içinde kilit tutulurken motoru
        // kapatmak, aynı kilidi bekleyen ses callback'iyle kilitlenmeye (deadlock) yol açıyordu.
        let stream_err = sh.error.lock().unwrap().take();
        if let Some(e) = stream_err {
            *app.error.lock().unwrap() = Some(e);
            *app.engine.lock().unwrap() = None;
            retry_at = Some(Instant::now() + Duration::from_secs(2));
            let _ = h.emit("status", app.status());
        }
        let running = app.engine.lock().unwrap().is_some();
        if !running && retry_at.is_none() {
            retry_at = Some(Instant::now() + Duration::from_secs(4));
        }
        if let Some(t) = retry_at {
            if Instant::now() >= t {
                retry_at = None;
                if !app.busy.load(Relaxed) {
                    app.restart();
                    let _ = h.emit("status", app.status());
                }
            }
        }

        if !app.ui_active.load(Relaxed) {
            continue; // pencere gizli: kimse bakmıyor, çizim verisi üretme
        }
        let (raw, out) = match app.spec.lock().unwrap().as_mut() {
            Some((r, o)) => {
                let (a, b) = scope.update(r, o);
                (Some(a), Some(b))
            }
            _ => (None, None),
        };
        let _ = h.emit(
            "meters",
            json!({
                "in_db": lin_to_db(ftake(&sh.m_in)),
                "out_db": lin_to_db(ftake(&sh.m_out)),
                "gr_db": ftake(&sh.m_gr),
                "vad": ftake(&sh.m_vad),
                "cpu": if tick % 15 == 0 { Some(ftake(&sh.cpu)) } else { None },
                "buf_ms": fload(&sh.buf_ms),
                "underruns": sh.underruns.load(Relaxed),
                "mon_underruns": sh.mon_underruns.load(Relaxed),
                "eff_ms": fload(&sh.eff_ms),
                "mon_eff_ms": fload(&sh.mon_eff_ms),
                "clips": sh.clips.load(Relaxed),
                "wave_raw": raw,
                "wave_out": out,
            }),
        );
    }
}

fn saver_loop(app: Arc<App>) {
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if app.dirty.swap(false, Relaxed) {
            app.save();
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("settings.json");
            let mut settings: Settings = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            settings.pads.resize(PAD_COUNT, None);
            settings.dsp.mute = false;

            let shared = Arc::new(Shared::new(settings.dsp));
            let pad_infos = settings.pads.clone();
            let st = Arc::new(App {
                shared,
                settings: Mutex::new(settings),
                engine: Mutex::new(None),
                info: Mutex::new(None),
                error: Mutex::new(None),
                sb_tx: Mutex::new(None),
                mon_tx: Mutex::new(None),
                rec: Mutex::new(None),
                spec: Mutex::new(None),
                pads: Mutex::new(vec![None; PAD_COUNT]),
                test: Mutex::new(None),
                busy: AtomicBool::new(false),
                dirty: AtomicBool::new(false),
                ui_active: AtomicBool::new(true),
                path,
            });
            {
                let s = st.settings.lock().unwrap().clone();
                st.commit(&s);
            }
            app.manage(st.clone());
            st.restart();

            // kayıtlı soundboard seslerini arka planda yükle
            let st2 = st.clone();
            std::thread::spawn(move || {
                for (i, p) in pad_infos.iter().enumerate() {
                    if let Some(p) = p {
                        if let Ok(buf) = sound::load(&p.path) {
                            st2.pads.lock().unwrap()[i] = Some(Arc::new(buf));
                        }
                    }
                }
            });

            let h = app.handle().clone();
            let st3 = st.clone();
            std::thread::spawn(move || meter_loop(h, st3));
            let st4 = st.clone();
            std::thread::spawn(move || saver_loop(st4));

            setup_tray(app)?;
            setup_hotkeys(app)?;

            if !std::env::args().any(|a| a == "--minimized") {
                show_main(app.handle());
            }
            Ok(())
        })
        .on_window_event(|w, e| {
            // pencere kapatılınca tepsiye küçül, ses çalışmaya devam etsin
            let app = w.state::<Arc<App>>();
            match e {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = w.hide();
                    app.ui_active.store(false, Relaxed);
                }
                // küçültme/geri yükleme Resized olayıyla gelir
                WindowEvent::Resized(_) | WindowEvent::Focused(_) => {
                    let minimized = w.is_minimized().unwrap_or(false);
                    let visible = w.is_visible().unwrap_or(true);
                    app.ui_active.store(visible && !minimized, Relaxed);
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            list_devices,
            set_dsp,
            set_preset,
            set_devices,
            restart_engine,
            set_hide_capture,
            assign_pad,
            clear_pad,
            play_pad,
            stop_pads,
            diag_run,
            test_record,
            test_play,
            test_stop,
            set_autostart,
            set_ui_active,
            save_custom,
            apply_custom,
            delete_custom,
            rename_custom,
            update_custom,
        ])
        .run(tauri::generate_context!())
        .expect("CleanMic başlatılamadı");
}
