//! Settings in `%LOCALAPPDATA%\TapFeed\config.json` (first start copies the old Tapo-Viewer one).
//! Unknown keys are kept so both versions can share it.

use serde_json::{json, Map, Value};
use std::path::PathBuf;

fn path() -> PathBuf {
    crate::runtime::data_dir().join("config.json")
}

fn defaults() -> Map<String, Value> {
    let v = json!({
        "host": "",
        "username": "",
        "save_credentials": false,
        "output_dir": crate::runtime::default_recordings_dir().to_string_lossy(),
        "live_stream_quality": "HD",
        "live_stream_muted": true,
        "live_stream_volume": 50,
        "language": "en",
        "check_updates": true,
        "skipped_version": "",
        "login_method": "",
        "talk_mic_id": "",
    });
    v.as_object().cloned().unwrap()
}

pub fn load() -> Map<String, Value> {
    let mut cfg = defaults();
    if let Ok(text) = std::fs::read_to_string(path()) {
        if let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(&text) {
            for (k, v) in saved {
                cfg.insert(k, v);
            }
        }
    }
    cfg
}

/// Merge `patch` into the saved settings and write them back.
pub fn update(patch: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut cfg = load();
    for (k, v) in patch {
        cfg.insert(k.clone(), v.clone());
    }
    let text = serde_json::to_string_pretty(&Value::Object(cfg.clone())).map_err(|e| e.to_string())?;
    let p = path();
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
    Ok(cfg)
}

pub fn get_str(cfg: &Map<String, Value>, key: &str) -> String {
    cfg.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn output_dir() -> PathBuf {
    let cfg = load();
    let dir = get_str(&cfg, "output_dir");
    if dir.trim().is_empty() {
        crate::runtime::default_recordings_dir()
    } else {
        PathBuf::from(dir)
    }
}
