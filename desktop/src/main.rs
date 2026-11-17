// TapFeed (formerly Tapo-Viewer): Tauri 2 shell, libmpv video, pytapo through a bundled Python helper.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod applog;
mod config;
mod creds;
mod helper;
mod mpv;
mod runtime;
mod update;
mod video_host;

use helper::{Helper, HelperError};
use mpv::Mpv;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------

struct Session {
    host: String,
    username: String,
    password: String,
}

#[derive(Clone, PartialEq)]
enum Media {
    None,
    Live,
    Clip(String),
}

#[derive(Clone, Serialize)]
struct Download {
    key: String,
    start: i64,
    end: i64,
    path: String,
    /// queued | downloading | done | failed | cancelled
    state: String,
    progress: f64,
    error: Option<HelperError>,
}

struct App {
    helper: Helper,
    pytapo: Mutex<String>,
    session: Mutex<Option<Session>>,
    player: Mutex<Option<Arc<Mpv>>>,
    player_error: Mutex<Option<String>>,
    host_hwnd: Mutex<Option<isize>>,
    media: Mutex<Media>,
    /// Live view was stopped for a download; resume it when the queue is empty.
    live_paused: Mutex<bool>,
    downloads: Mutex<Vec<Download>>,
    wake: tokio::sync::Notify,
}

type Res<T> = Result<T, HelperError>;

fn err(code: &str, msg: impl Into<String>) -> HelperError {
    HelperError::new(code, msg)
}

const SHORT: Duration = Duration::from_secs(60);
/// Two login attempts at most; a reachable camera answers well within this.
const CONNECT: Duration = Duration::from_secs(40);
const LONG: Duration = Duration::from_secs(4 * 60 * 60);

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Percent-encode everything outside RFC 3986 "unreserved" (for RTSP user/password).
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn clip_key(start: i64, end: i64) -> String {
    format!("{start}-{end}")
}

/// `tapo_YYYYMMDD_HHMMSS_HHMMSS.ts`, local time, the naming the Python app used.
fn clip_base(start: i64, end: i64) -> String {
    use chrono::{Local, TimeZone};
    let s = Local.timestamp_opt(start, 0).single().unwrap_or_default();
    let e = Local.timestamp_opt(end, 0).single().unwrap_or_default();
    format!("tapo_{}_{}_{}", s.format("%Y%m%d"), s.format("%H%M%S"), e.format("%H%M%S"))
}

/// An existing download of this clip (.ts from this app, .mp4 from the Python one).
/// A `.part` file next to it means the download never finished.
fn existing_file(dir: &Path, start: i64, end: i64) -> Option<PathBuf> {
    let base = clip_base(start, end);
    ["ts", "mp4"].iter().map(|x| dir.join(format!("{base}.{x}"))).find(|p| {
        p.metadata().map(|m| m.len() > 1024).unwrap_or(false) && !part_marker(p).exists()
    })
}

fn part_marker(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn player(app: &App) -> Res<Arc<Mpv>> {
    app.player.lock().unwrap().clone().ok_or_else(|| {
        let detail = app.player_error.lock().unwrap().clone().unwrap_or_else(|| "starting".into());
        err("ERR_PLAYER_STARTING", detail)
    })
}

fn shell_open(target: &str, args: Option<&str>) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, file) = (w("open"), w(target));
    let params = args.map(w);
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn emit_downloads(handle: &AppHandle) {
    let app = handle.state::<App>();
    let list = app.downloads.lock().unwrap().clone();
    let _ = handle.emit("downloads", list);
}

/// Accept a bare IP or host name; reject URLs, ports and spaces.
fn clean_host(host: &str) -> Res<String> {
    let h = host.trim();
    let ok = !h.is_empty()
        && h.len() <= 253
        && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if ok { Ok(h.to_string()) } else { Err(err("ERR_BAD_HOST", h)) }
}

/// Is a TCP port open? Quick, so a wrong address fails in seconds, not minutes.
fn port_open(host: &str, port: u16, timeout: Duration) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let Ok(addrs) = (host, port).to_socket_addrs() else { return false };
    addrs.into_iter().any(|a| TcpStream::connect_timeout(&a, timeout).is_ok())
}

async fn probe(host: String) -> (bool, bool) {
    let h2 = host.clone();
    let t = Duration::from_millis(2500);
    let control = tauri::async_runtime::spawn_blocking(move || port_open(&host, 443, t));
    let video = tauri::async_runtime::spawn_blocking(move || port_open(&h2, 554, t));
    (control.await.unwrap_or(false), video.await.unwrap_or(false))
}

/// The Test button: can this PC reach a camera at that address?
#[tauri::command]
async fn test_camera(host: String) -> Res<Value> {
    let host = clean_host(&host)?;
    let (control, video) = probe(host).await;
    Ok(json!({ "control": control, "video": video }))
}

// ---------------------------------------------------------------------------
// app, settings, login
// ---------------------------------------------------------------------------

#[tauri::command]
async fn app_info(app: State<'_, App>) -> Result<Value, ()> {
    if app.pytapo.lock().unwrap().is_empty() {
        if let Ok(v) = app.helper.call("hello", json!({}), SHORT).await {
            *app.pytapo.lock().unwrap() = v["pytapo"].as_str().unwrap_or_default().to_string();
        }
    }
    let libmpv = app.player.lock().unwrap().as_ref().and_then(|p| p.get("mpv-version"));
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pytapo": *app.pytapo.lock().unwrap(),
        "libmpv": libmpv.map(|v| v.trim_start_matches("mpv ").to_string()),
        "dataDir": runtime::data_dir(),
        "logsDir": runtime::logs_dir(),
    }))
}

#[tauri::command]
fn settings_get() -> Map<String, Value> {
    config::load()
}

#[tauri::command]
fn settings_set(patch: Map<String, Value>) -> Res<Map<String, Value>> {
    let allowed = [
        "language", "check_updates", "skipped_version", "live_stream_quality",
        "live_stream_muted", "live_stream_volume",
    ];
    let clean: Map<String, Value> = patch.into_iter().filter(|(k, _)| allowed.contains(&k.as_str())).collect();
    config::update(&clean).map_err(|e| err("ERR_SETTINGS", e))
}

/// Choose the download folder after checking it can be written to.
#[tauri::command]
fn set_output_dir(path: String) -> Res<Map<String, Value>> {
    let dir = PathBuf::from(&path);
    std::fs::create_dir_all(&dir).map_err(|e| err("ERR_STORAGE_PERM", e.to_string()))?;
    let probe = dir.join(".tapfeed-write-test");
    std::fs::write(&probe, b"ok").map_err(|e| err("ERR_STORAGE_PERM", e.to_string()))?;
    let _ = std::fs::remove_file(probe);
    let mut patch = Map::new();
    patch.insert("output_dir".into(), json!(path));
    config::update(&patch).map_err(|e| err("ERR_SETTINGS", e))
}

#[tauri::command]
fn saved_login() -> Value {
    let cfg = config::load();
    let host = config::get_str(&cfg, "host");
    let user = config::get_str(&cfg, "username");
    let remember = cfg.get("save_credentials").and_then(Value::as_bool).unwrap_or(false);
    // Saved passwords never go to the page; it only learns that they exist.
    let (mut has_pw, mut has_cloud) = (false, false);
    if remember && !host.is_empty() && !user.is_empty() {
        has_pw = creds::load(&creds::account("camera", &host, &user)).is_some();
        has_cloud = creds::load(&creds::account("cloud", &host, &user)).is_some();
    }
    json!({ "host": host, "username": user, "remember": remember, "hasPassword": has_pw, "hasCloudPassword": has_cloud })
}

#[tauri::command]
async fn connect(
    app: State<'_, App>,
    host: String,
    username: String,
    password: String,
    cloud_password: Option<String>,
    remember: bool,
) -> Res<Value> {
    let host = clean_host(&host)?;
    let username = username.trim().to_string();
    // An empty field means "use the password saved on this PC", if there is one.
    let password = if password.is_empty() {
        creds::load(&creds::account("camera", &host, &username))
            .ok_or_else(|| err("ERR_INPUT", "password missing"))?
    } else {
        password
    };
    let cloud = match cloud_password {
        Some(c) if c.is_empty() => creds::load(&creds::account("cloud", &host, &username)),
        other => other,
    };
    // Fail fast on a wrong address instead of waiting out pytapo's retries.
    let (control, _video) = probe(host.clone()).await;
    if !control {
        return Err(err("ERR_CAMERA_UNREACHABLE", format!("nothing answered on {host}:443")));
    }
    let cfg = config::load();
    let same_camera = config::get_str(&cfg, "host") == host && config::get_str(&cfg, "username") == username;
    let prefer = if same_camera { config::get_str(&cfg, "login_method") } else { String::new() };

    let info = app
        .helper
        .call(
            "connect",
            json!({ "host": host, "username": username, "password": password,
                    "cloud_password": cloud, "prefer": prefer }),
            CONNECT,
        )
        .await?;
    applog::write(&format!("signed in to {host} using the {} login", info["login"].as_str().unwrap_or("?")));

    let mut patch = Map::new();
    patch.insert("host".into(), json!(host));
    patch.insert("username".into(), json!(username));
    patch.insert("save_credentials".into(), json!(remember));
    patch.insert("login_method".into(), info["login"].clone());
    let _ = config::update(&patch);

    let (cam_acc, cloud_acc) = (creds::account("camera", &host, &username), creds::account("cloud", &host, &username));
    let mut warning = None;
    if remember {
        let saved = creds::save(&cam_acc, &password).and_then(|_| match &cloud {
            Some(c) => creds::save(&cloud_acc, c),
            None => {
                creds::remove(&cloud_acc);
                Ok(())
            }
        });
        if let Err(e) = saved {
            applog::write(&format!("could not save password: {e}"));
            warning = Some("ERR_CREDENTIAL_STORE");
        }
    } else {
        creds::remove(&cam_acc);
        creds::remove(&cloud_acc);
    }

    *app.session.lock().unwrap() = Some(Session { host, username, password });
    let mut out = info;
    out["warning"] = json!(warning);
    Ok(out)
}

#[tauri::command]
async fn disconnect(handle: AppHandle, app: State<'_, App>) -> Res<()> {
    if let Ok(p) = player(&app) {
        let _ = p.command(&["stop"]);
    }
    *app.media.lock().unwrap() = Media::None;
    *app.live_paused.lock().unwrap() = false;
    *app.session.lock().unwrap() = None;
    {
        let mut list = app.downloads.lock().unwrap();
        for d in list.iter_mut().filter(|d| d.state == "queued") {
            d.state = "cancelled".into();
        }
    }
    app.helper.send("cancel");
    let _ = app.helper.call("disconnect", json!({}), SHORT).await;
    emit_downloads(&handle);
    Ok(())
}

// ---------------------------------------------------------------------------
// recordings
// ---------------------------------------------------------------------------

#[tauri::command]
async fn recording_dates(app: State<'_, App>) -> Res<Value> {
    app.helper.call("recording_dates", json!({}), SHORT).await
}

#[tauri::command]
async fn list_recordings(app: State<'_, App>, date: String) -> Res<Vec<Value>> {
    let raw = app.helper.call("list_recordings", json!({ "date": date }), SHORT).await?;
    let dir = config::output_dir();
    let downloads = app.downloads.lock().unwrap().clone();
    Ok(raw
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            let (s, e) = (c["start"].as_i64().unwrap_or(0), c["end"].as_i64().unwrap_or(0));
            let key = clip_key(s, e);
            let file = existing_file(&dir, s, e);
            let dl = downloads.iter().rev().find(|d| d.key == key);
            json!({
                "key": key, "start": s, "end": e, "motion": c["motion"], "types": c["types"],
                "file": file,
                "download": dl.map(|d| json!({"state": d.state, "progress": d.progress})),
            })
        })
        .collect())
}

/// Data URLs, null where the camera has no image.
#[tauri::command]
async fn clip_thumbnails(app: State<'_, App>, starts: Vec<i64>) -> Res<Value> {
    let host = app.session.lock().unwrap().as_ref().map(|s| s.host.clone())
        .ok_or_else(|| err("ERR_NOT_CONNECTED", "not signed in"))?;
    let dir = runtime::thumbs_dir().join(host);
    app.helper.call("thumbnails", json!({ "starts": starts, "dir": dir }), SHORT).await
}

// ---------------------------------------------------------------------------
// pan / tilt (only when the camera reports a motor)
// ---------------------------------------------------------------------------

const PTZ: Duration = Duration::from_secs(15);

#[tauri::command]
async fn ptz_info(app: State<'_, App>) -> Res<Value> {
    app.helper.call("ptz_info", json!({}), SHORT).await
}

#[tauri::command]
async fn ptz_move(app: State<'_, App>, dx: i32, dy: i32) -> Res<Value> {
    app.helper.call("ptz_move", json!({ "dx": dx.clamp(-30, 30), "dy": dy.clamp(-30, 30) }), PTZ).await
}

#[tauri::command]
async fn ptz_goto(app: State<'_, App>, id: String) -> Res<Value> {
    app.helper.call("ptz_goto", json!({ "id": id }), PTZ).await
}

// ---------------------------------------------------------------------------
// player
// ---------------------------------------------------------------------------

/// The page reports where the video box is (physical pixels). Runs on the UI thread.
#[tauri::command]
fn set_video_rect(window: WebviewWindow, app: State<App>, x: i32, y: i32, w: i32, h: i32) -> Res<()> {
    let mut host = app.host_hwnd.lock().unwrap();
    if let Some(hwnd) = *host {
        video_host::place(hwnd, x, y, w, h);
        return Ok(());
    }
    let parent = match window.window_handle().map_err(|e| err("ERR_PLAYER_START", e.to_string()))?.as_raw() {
        RawWindowHandle::Win32(h) => h.hwnd.get(),
        _ => return Err(err("ERR_PLAYER_START", "not a Win32 window")),
    };
    let hwnd = video_host::create(parent, x, y, w, h).map_err(|e| err("ERR_PLAYER_START", e))?;
    *host = Some(hwnd);
    let made = runtime::libmpv().and_then(|dll| Mpv::new(&dll, hwnd));
    match made {
        Ok(p) => {
            let cfg = config::load();
            let vol = cfg.get("live_stream_volume").and_then(Value::as_i64).unwrap_or(50);
            let muted = cfg.get("live_stream_muted").and_then(Value::as_bool).unwrap_or(true);
            let _ = p.set("volume", &vol.to_string());
            let _ = p.set("mute", if muted { "yes" } else { "no" });
            *app.player.lock().unwrap() = Some(p);
            Ok(())
        }
        Err(e) => {
            applog::write(&format!("video player failed to start: {e}"));
            *app.player_error.lock().unwrap() = Some(e.clone());
            Err(err("ERR_PLAYER_START", e))
        }
    }
}

/// Hide the native video window while another page or a dialog is showing.
#[tauri::command]
fn set_video_visible(app: State<App>, visible: bool) {
    if let Some(hwnd) = *app.host_hwnd.lock().unwrap() {
        video_host::show(hwnd, visible);
    }
}

/// Live camera feed: show each frame as soon as it arrives. Tapo RTSP timestamps jump
/// (worst while the camera turns), and waiting for audio sync froze the picture.
const LIVE_TUNING: &[(&str, &str)] = &[
    ("untimed", "yes"),
    ("video-latency-hacks", "yes"),
    ("framedrop", "vo"),
    ("cache", "no"),
    ("demuxer-readahead-secs", "0"),
    ("demuxer-lavf-o", "fflags=+nobuffer"),
    ("demuxer-lavf-analyzeduration", "0.5"),
    ("demuxer-lavf-probe-info", "nostreams"),
    ("demuxer-lavf-probesize", "0"),
];

/// Recorded clips: normal playback with exact audio sync and seeking.
const CLIP_TUNING: &[(&str, &str)] = &[
    ("untimed", "no"),
    ("video-latency-hacks", "no"),
    ("framedrop", "vo"),
    ("cache", "auto"),
    ("demuxer-readahead-secs", "1"),
    ("demuxer-lavf-o", ""),
    ("demuxer-lavf-analyzeduration", "0"),
    ("demuxer-lavf-probe-info", "auto"),
    // ffmpeg cannot identify the camera's audio (0x90) and would read 5 MB (~20 s) first.
    ("demuxer-lavf-probesize", "524288"),
];

fn tune(p: &Mpv, settings: &[(&str, &str)]) {
    for (k, v) in settings {
        if let Err(e) = p.set(k, v) {
            applog::write(&format!("player setting {k}={v} not applied: {e}"));
        }
    }
}

#[tauri::command]
fn start_live(app: State<App>, quality: String) -> Res<()> {
    *app.live_paused.lock().unwrap() = false;
    load_live(&app, &quality)
}

fn load_live(app: &App, quality: &str) -> Res<()> {
    let p = player(app)?;
    let url = {
        let s = app.session.lock().unwrap();
        let s = s.as_ref().ok_or_else(|| err("ERR_NOT_CONNECTED", "not signed in"))?;
        let stream = if quality == "SD" { 2 } else { 1 };
        format!("rtsp://{}:{}@{}:554/stream{stream}", enc(&s.username), enc(&s.password), s.host)
    };
    let mut patch = Map::new();
    patch.insert("live_stream_quality".into(), json!(quality));
    let _ = config::update(&patch);
    p.clear_log();
    p.set("pause", "no").map_err(|e| err("ERR_PLAYBACK", e))?;
    tune(&p, LIVE_TUNING);
    p.command(&["loadfile", &url])
        .map_err(|e| err("ERR_LIVE_REJECTED", e))?;
    *app.media.lock().unwrap() = Media::Live;
    Ok(())
}

/// `growing`: still downloading; mpv keeps reading as it grows.
#[tauri::command]
fn play_file(app: State<App>, path: String, growing: Option<bool>) -> Res<()> {
    if !Path::new(&path).exists() {
        return Err(err("ERR_FILE_MISSING", path));
    }
    let p = player(&app)?;
    p.clear_log();
    p.set("pause", "no").map_err(|e| err("ERR_PLAYBACK", e))?;
    tune(&p, CLIP_TUNING);
    let url = if growing == Some(true) { format!("appending://{path}") } else { path.clone() };
    p.command(&["loadfile", &url]).map_err(|e| err("ERR_PLAYBACK_OPEN", e))?;
    *app.live_paused.lock().unwrap() = false;
    *app.media.lock().unwrap() = Media::Clip(path);
    Ok(())
}

#[tauri::command]
fn player_control(app: State<App>, action: String, value: Option<String>) -> Res<()> {
    let p = player(&app)?;
    let r = match action.as_str() {
        "stop" => {
            *app.media.lock().unwrap() = Media::None;
            *app.live_paused.lock().unwrap() = false;
            p.command(&["stop"])
        }
        "pause" => p.command(&["cycle", "pause"]),
        "seek" => p.command(&["seek", value.as_deref().unwrap_or("10"), "relative"]),
        "seek_to" => p.command(&["seek", value.as_deref().unwrap_or("0"), "absolute"]),
        "mute" => {
            let r = p.command(&["cycle", "mute"]);
            let mut patch = Map::new();
            patch.insert("live_stream_muted".into(), json!(p.get("mute").as_deref() == Some("yes")));
            let _ = config::update(&patch);
            r
        }
        "volume" => {
            let v: i64 = value.as_deref().unwrap_or("50").parse().unwrap_or(50).clamp(0, 100);
            let mut patch = Map::new();
            patch.insert("live_stream_volume".into(), json!(v));
            let _ = config::update(&patch);
            p.set("volume", &v.to_string())
        }
        other => return Err(err("ERR_INPUT", format!("unknown action {other}"))),
    };
    r.map_err(|e| err("ERR_PLAYBACK", e))
}

/// Save the current frame as a PNG in the downloads folder.
#[tauri::command]
fn take_screenshot(app: State<App>) -> Res<String> {
    let p = player(&app)?;
    if *app.media.lock().unwrap() == Media::None {
        return Err(err("ERR_NOTHING_PLAYING", "nothing is playing"));
    }
    let dir = config::output_dir();
    std::fs::create_dir_all(&dir).map_err(|e| err("ERR_STORAGE_PERM", e.to_string()))?;
    let name = format!("tapo_snapshot_{}.png", chrono::Local::now().format("%Y%m%d_%H%M%S"));
    let path = dir.join(name).to_string_lossy().to_string();
    p.command(&["screenshot-to-file", &path, "video"]).map_err(|e| err("ERR_SCREENSHOT", e))?;
    Ok(path)
}

/// Open a downloaded recording in the PC's default video app.
#[tauri::command]
fn open_external(path: String) -> Res<()> {
    open_path(path)
}

/// Map libmpv/ffmpeg wording to an error code the UI explains.
fn classify(problems: &[String], live: bool) -> &'static str {
    let all = problems.join(" ").to_lowercase();
    if all.contains("401") || all.contains("unauthorized") {
        "ERR_RTSP_AUTH"
    } else if all.contains("timed out") || all.contains("timeout") {
        "ERR_CAMERA_TIMEOUT"
    } else if all.contains("refused") {
        "ERR_RTSP_REFUSED"
    } else if all.contains("unreachable") || all.contains("no route") {
        "ERR_CAMERA_UNREACHABLE"
    } else if live {
        "ERR_LIVE_REJECTED"
    } else {
        "ERR_PLAYBACK_OPEN"
    }
}

#[tauri::command]
fn player_status(app: State<App>) -> Value {
    let Ok(p) = player(&app) else {
        let failed = app.player_error.lock().unwrap().is_some();
        return json!({ "ready": false, "failed": failed });
    };
    let media = app.media.lock().unwrap().clone();
    let ev = p.last_event.lock().unwrap().clone();
    let problems: Vec<String> = p.log.lock().unwrap().iter().cloned().collect();
    let live = media == Media::Live;
    let num = |k: &str| p.get(k).and_then(|v| v.parse::<f64>().ok());
    json!({
        "ready": true,
        "livePaused": *app.live_paused.lock().unwrap(),
        "mode": match &media { Media::None => "none", Media::Live => "live", Media::Clip(_) => "clip" },
        "path": match &media { Media::Clip(p) => Some(p.clone()), _ => None },
        "state": ev.state,
        "error": ev.error.as_ref().map(|_| classify(&problems, live)),
        "detail": ev.error.map(|e| format!("{e}. {}", problems.join(" | "))),
        "position": num("time-pos"),
        "duration": num("duration"),
        "paused": p.get("pause").as_deref() == Some("yes"),
        "buffering": p.get("paused-for-cache").as_deref() == Some("yes"),
        "muted": p.get("mute").as_deref() == Some("yes"),
        "volume": num("volume"),
        "width": num("width"),
        "height": num("height"),
    })
}

// ---------------------------------------------------------------------------
// downloads
// ---------------------------------------------------------------------------

/// `first`: ahead of waiting downloads.
#[tauri::command]
fn queue_download(handle: AppHandle, app: State<App>, start: i64, end: i64, first: Option<bool>) -> Res<Download> {
    let key = clip_key(start, end);
    let dir = config::output_dir();
    let path = dir.join(format!("{}.ts", clip_base(start, end)));
    let item = {
        let mut list = app.downloads.lock().unwrap();
        if let Some(d) = list.iter().find(|d| d.key == key && (d.state == "queued" || d.state == "downloading")) {
            return Ok(d.clone());
        }
        list.retain(|d| d.key != key);
        let item = Download {
            key, start, end,
            path: path.to_string_lossy().to_string(),
            state: "queued".into(),
            progress: 0.0,
            error: None,
        };
        let at = match first {
            Some(true) => list.iter().position(|d| d.state == "queued").unwrap_or(list.len()),
            _ => list.len(),
        };
        list.insert(at, item.clone());
        item
    };
    app.wake.notify_one();
    emit_downloads(&handle);
    Ok(item)
}

#[tauri::command]
fn cancel_download(handle: AppHandle, app: State<App>, key: String) {
    let mut running = false;
    {
        let mut list = app.downloads.lock().unwrap();
        if let Some(d) = list.iter_mut().find(|d| d.key == key) {
            match d.state.as_str() {
                "queued" => d.state = "cancelled".into(),
                "downloading" => running = true,
                _ => {}
            }
        }
    }
    if running {
        app.helper.send("cancel");
    }
    emit_downloads(&handle);
}

#[tauri::command]
fn downloads(app: State<App>) -> Vec<Download> {
    app.downloads.lock().unwrap().clone()
}

#[tauri::command]
fn clear_finished_downloads(handle: AppHandle, app: State<App>) {
    app.downloads.lock().unwrap().retain(|d| d.state == "queued" || d.state == "downloading");
    emit_downloads(&handle);
}

/// One download at a time: the camera serves a single recording stream.
async fn download_worker(handle: AppHandle) {
    loop {
        let next = {
            let app = handle.state::<App>();
            let mut list = app.downloads.lock().unwrap();
            list.iter_mut().find(|d| d.state == "queued").map(|d| {
                d.state = "downloading".into();
                d.clone()
            })
        };
        let Some(job) = next else {
            resume_live(&handle);
            handle.state::<App>().wake.notified().await;
            continue;
        };
        pause_live(&handle);
        emit_downloads(&handle);

        let result = handle
            .state::<App>()
            .helper
            .call("download", json!({ "start": job.start, "end": job.end, "path": job.path }), LONG)
            .await;
        let failed = result.is_err();
        {
            let app = handle.state::<App>();
            let mut list = app.downloads.lock().unwrap();
            if let Some(d) = list.iter_mut().find(|d| d.key == job.key) {
                match result {
                    Ok(_) => {
                        d.state = "done".into();
                        d.progress = 1.0;
                    }
                    Err(e) if e.code == "ERR_DOWNLOAD_CANCELLED" => d.state = "cancelled".into(),
                    Err(e) => {
                        applog::write(&format!("download {} failed: {} {}", job.key, e.code, e.message));
                        d.state = "failed".into();
                        d.error = Some(e);
                    }
                }
            }
        }
        if failed {
            discard_partial(&handle, &job.path);
        }
        emit_downloads(&handle);
    }
}

/// The camera is too slow to serve live view and a download at once.
fn pause_live(handle: &AppHandle) {
    let app = handle.state::<App>();
    let mut media = app.media.lock().unwrap();
    if *media == Media::Live {
        if let Ok(p) = player(&app) {
            let _ = p.command(&["stop"]);
        }
        *media = Media::None;
        *app.live_paused.lock().unwrap() = true;
    }
}

fn resume_live(handle: &AppHandle) {
    let app = handle.state::<App>();
    if std::mem::take(&mut *app.live_paused.lock().unwrap()) && *app.media.lock().unwrap() == Media::None {
        let quality = config::get_str(&config::load(), "live_stream_quality");
        if let Err(e) = load_live(&app, &quality) {
            applog::write(&format!("could not resume live view: {} {}", e.code, e.message));
        }
    }
}

/// Delete an unfinished download, stopping its playback first.
fn discard_partial(handle: &AppHandle, path: &str) {
    let app = handle.state::<App>();
    let mut media = app.media.lock().unwrap();
    if *media == Media::Clip(path.to_string()) {
        if let Ok(p) = player(&app) {
            let _ = p.command(&["stop"]);
        }
        *media = Media::None;
    }
    drop(media);
    let path = Path::new(path);
    // mpv may still hold the file; a leftover keeps its .part marker.
    for _ in 0..10 {
        if std::fs::remove_file(path).is_ok() || !path.exists() {
            let _ = std::fs::remove_file(part_marker(path));
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn on_helper_event(handle: &AppHandle, ev: Value) {
    if ev["event"] != "download_progress" {
        return;
    }
    let (p, t) = (ev["progress"].as_f64().unwrap_or(0.0), ev["total"].as_f64().unwrap_or(0.0));
    let app = handle.state::<App>();
    let changed = {
        let mut list = app.downloads.lock().unwrap();
        match list.iter_mut().find(|d| d.state == "downloading") {
            Some(d) if t > 0.0 => {
                d.progress = (p / t).clamp(0.0, 0.99);
                true
            }
            _ => false,
        }
    };
    if changed {
        emit_downloads(handle);
    }
}

// ---------------------------------------------------------------------------
// files, links, updates, languages, logging
// ---------------------------------------------------------------------------

#[tauri::command]
fn open_path(path: String) -> Res<()> {
    if !Path::new(&path).exists() {
        return Err(err("ERR_FILE_MISSING", path));
    }
    shell_open(&path, None);
    Ok(())
}

#[tauri::command]
fn reveal_path(path: String) {
    if Path::new(&path).is_file() {
        shell_open("explorer.exe", Some(&format!("/select,\"{path}\"")));
    } else {
        let _ = std::fs::create_dir_all(&path);
        shell_open(&path, None);
    }
}

#[tauri::command]
fn output_dir() -> String {
    config::output_dir().to_string_lossy().to_string()
}

/// Only the project's own pages open from the app.
#[tauri::command]
fn open_link(url: String) -> Res<()> {
    let ok = [
        "https://github.com/zaifears/tapo-viewer",
        "https://github.com/zaifears/TapFeed",
        "https://github.com/zaifears/tapfeed",
        "https://github.com/JurajNyiri/pytapo",
        "https://mpv.io",
        "https://www.videolan.org/vlc",
        "https://shahoriar.bd/thanks",
    ]
        .iter()
        .any(|p| url.starts_with(p));
    if !ok {
        return Err(err("ERR_INPUT", "link not allowed"));
    }
    shell_open(&url, None);
    Ok(())
}

#[tauri::command]
async fn check_update(manual: bool) -> Res<Option<update::Release>> {
    let cfg = config::load();
    if !manual && cfg.get("check_updates").and_then(Value::as_bool) == Some(false) {
        return Ok(None);
    }
    let rel = tauri::async_runtime::spawn_blocking(update::latest)
        .await
        .map_err(|e| err("ERR_UPDATE_CHECK", e.to_string()))?
        .map_err(|e| err("ERR_UPDATE_CHECK", e))?;
    Ok(update::policy(rel, manual, &config::get_str(&cfg, "skipped_version")))
}

/// Language files dropped into %LOCALAPPDATA%\TapFeed\locales (no rebuild needed).
#[tauri::command]
fn user_locales() -> Map<String, Value> {
    let mut out = Map::new();
    if let Ok(entries) = std::fs::read_dir(runtime::user_locales_dir()) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let code = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
            if code.starts_with('_') {
                continue;
            }
            if let Ok(Value::Object(m)) = std::fs::read_to_string(&p).map(|t| serde_json::from_str(&t).unwrap_or(Value::Null)) {
                out.insert(code, Value::Object(m));
            }
        }
    }
    out
}

/// Open the user language folder, with a copy of English to translate.
#[tauri::command]
fn open_locales_folder() {
    let dir = runtime::user_locales_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("_template.json"), include_str!("../ui/locales/en.json"));
    shell_open(&dir.to_string_lossy(), None);
}

#[tauri::command]
fn log_line(line: String) {
    applog::write(&line);
}

// ---------------------------------------------------------------------------

fn main() {
    // Before anything reads settings: bring them over from the old name once.
    let migration = runtime::migrate_from_tapo_viewer();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            let h2 = handle.clone();
            let helper = Helper::new(
                Arc::new(move |ev| on_helper_event(&h2, ev)),
                Arc::new(|line: &str| applog::write(line)),
            );
            app.manage(App {
                helper,
                pytapo: Mutex::new(String::new()),
                session: Mutex::new(None),
                player: Mutex::new(None),
                player_error: Mutex::new(None),
                host_hwnd: Mutex::new(None),
                media: Mutex::new(Media::None),
                live_paused: Mutex::new(false),
                downloads: Mutex::new(Vec::new()),
                wake: tokio::sync::Notify::new(),
            });
            applog::write(concat!("TapFeed ", env!("CARGO_PKG_VERSION"), " started"));
            for note in migration.iter() {
                applog::write(&format!("rename carry-over: {note}"));
            }
            // Unpack and start the helper while the user types the password.
            let h3 = handle.clone();
            std::thread::spawn(move || {
                runtime::prune_old_runtimes();
                runtime::prune_old_thumbs();
                if let Err(e) = runtime::libmpv() {
                    applog::write(&format!("could not unpack libmpv: {e}"));
                }
                h3.state::<App>().helper.warm_up();
            });
            tauri::async_runtime::spawn(download_worker(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info, settings_get, settings_set, set_output_dir, saved_login, connect, disconnect,
            recording_dates, list_recordings, clip_thumbnails, set_video_rect, set_video_visible, start_live,
            play_file, player_control, player_status, queue_download, cancel_download, downloads,
            clear_finished_downloads, open_path, reveal_path, output_dir, open_link, check_update,
            user_locales, open_locales_folder, log_line, test_camera, take_screenshot, open_external, ptz_info, ptz_move, ptz_goto
        ])
        .build(tauri::generate_context!())
        .expect("failed to start TapFeed")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                handle.state::<App>().helper.shutdown();
            }
        });
}
