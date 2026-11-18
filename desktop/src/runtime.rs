//! Paths, and unpacking the parts baked into the exe (libmpv, the pytapo helper).

use std::path::{Path, PathBuf};

static LIBMPV_ZST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libmpv-2.dll.zst"));
static HELPER_ZST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tapo-helper.zip.zst"));

fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// `%LOCALAPPDATA%\TapFeed`.
pub fn data_dir() -> PathBuf {
    let dir = local_app_data().join("TapFeed");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The folder used under the old name (Tapo-Viewer 1.x in Python, and 2.0 test builds).
pub fn old_data_dir() -> PathBuf {
    local_app_data().join("Tapo-Viewer")
}

/// First start after the rename: copy settings and language files from the old folder.
/// Nothing in the old folder is moved or deleted except our own unpacked runtime cache,
/// so the old app keeps working and downloaded recordings stay exactly where they are.
pub fn migrate_from_tapo_viewer() -> Vec<String> {
    let mut notes = Vec::new();
    let (old, new) = (old_data_dir(), data_dir());
    let new_cfg = new.join("config.json");
    let old_cfg = old.join("config.json");
    if new_cfg.exists() || !old_cfg.exists() {
        return notes;
    }
    let mut cfg = std::fs::read_to_string(&old_cfg)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    // Keep using the folder the recordings are already in.
    let has_dir = cfg.get("output_dir").and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false);
    if !has_dir {
        cfg.insert("output_dir".into(), serde_json::json!(old.join("recordings").to_string_lossy()));
    }
    cfg.insert("migrated_from".into(), serde_json::json!("Tapo-Viewer"));
    match serde_json::to_string_pretty(&serde_json::Value::Object(cfg)) {
        Ok(text) => match write_atomic(&new_cfg, text.as_bytes()) {
            Ok(()) => notes.push(format!("copied settings from {}", old_cfg.display())),
            Err(e) => notes.push(format!("could not copy settings: {e}")),
        },
        Err(e) => notes.push(format!("could not read old settings: {e}")),
    }
    if let Ok(entries) = std::fs::read_dir(old.join("locales")) {
        let target = new.join("locales");
        let _ = std::fs::create_dir_all(&target);
        for e in entries.flatten() {
            let to = target.join(e.file_name());
            if !to.exists() && std::fs::copy(e.path(), &to).is_ok() {
                notes.push(format!("copied language file {}", e.file_name().to_string_lossy()));
            }
        }
    }
    // Unpacked libmpv/helper from 2.0 test builds: a cache, re-created on demand.
    if std::fs::remove_dir_all(old.join("runtime")).is_ok() {
        notes.push("removed the old unpacked runtime cache".into());
    }
    notes
}

pub fn logs_dir() -> PathBuf {
    let d = data_dir().join("logs");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn default_recordings_dir() -> PathBuf {
    data_dir().join("recordings")
}

pub fn user_locales_dir() -> PathBuf {
    data_dir().join("locales")
}

fn runtime_dir(hash: &str) -> PathBuf {
    data_dir().join("runtime").join(hash)
}

fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = target.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(_) if target.exists() => {
            let _ = std::fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) => Err(format!("rename {}: {e}", target.display())),
    }
}

/// Unpack libmpv once per version; later starts reuse it.
pub fn libmpv() -> Result<PathBuf, String> {
    let dir = runtime_dir(env!("LIBMPV_HASH"));
    let dll = dir.join("libmpv-2.dll");
    if dll.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(dll);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bytes = zstd::stream::decode_all(LIBMPV_ZST).map_err(|e| e.to_string())?;
    write_atomic(&dll, &bytes)?;
    Ok(dll)
}

/// Unpack the frozen Python helper once per version; returns its exe.
pub fn helper_exe() -> Result<PathBuf, String> {
    let dir = runtime_dir(env!("HELPER_HASH")).join("helper");
    let exe = dir.join("tapo-helper.exe");
    let done = dir.join(".complete");
    if done.exists() && exe.exists() {
        return Ok(exe);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip_bytes = zstd::stream::decode_all(HELPER_ZST).map_err(|e| e.to_string())?;
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| e.to_string())?;
    archive.extract(&dir).map_err(|e| format!("unpack helper: {e}"))?;
    std::fs::write(&done, b"ok").map_err(|e| e.to_string())?;
    Ok(exe)
}

/// Remove unpacked parts left by older versions.
/// `cache\<camera>\clips` and `cache\<camera>\thumbs`.
pub fn cache_dir() -> PathBuf {
    data_dir().join("cache")
}

pub fn camera_cache_dir(host: &str) -> PathBuf {
    cache_dir().join(host)
}

fn cached_files(sub: &str) -> Vec<std::fs::DirEntry> {
    let Ok(cameras) = std::fs::read_dir(cache_dir()) else { return Vec::new() };
    cameras.flatten().filter_map(|c| std::fs::read_dir(c.path().join(sub)).ok()).flatten().flatten().collect()
}

/// The SD card has overwritten such recordings long before.
pub fn prune_old_thumbs() {
    let max_age = std::time::Duration::from_secs(60 * 24 * 60 * 60);
    for file in cached_files("thumbs") {
        let old = file.metadata().and_then(|m| m.modified()).ok()
            .and_then(|t| t.elapsed().ok()).is_some_and(|age| age > max_age);
        if old {
            let _ = std::fs::remove_file(file.path());
        }
    }
}

/// Delete least recently watched clips until the rest fit in `limit` bytes.
/// `at_start`: unfinished ones (.part) too, as nothing is downloading yet.
pub fn trim_cache(limit: u64, at_start: bool) {
    let mut clips = Vec::new();
    let mut total = 0;
    let mut files = cached_files("clips");
    // Markers last, so a clip is gone before its marker is.
    files.sort_by_key(|f| is_part_marker(&f.path()));
    for f in files {
        let path = f.path();
        let marker = is_part_marker(&path);
        let unfinished = marker || part_marker(&path).exists();
        if at_start && unfinished {
            remove_cached(&path);
            continue;
        }
        let Ok(meta) = f.metadata() else { continue };
        total += meta.len();
        if !unfinished {
            clips.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len(), path));
        }
    }
    clips.sort();
    for (_, len, path) in clips {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries.flatten().map(|e| match e.metadata() {
        Ok(m) if m.is_dir() => dir_size(&e.path()),
        Ok(m) => m.len(),
        Err(_) => 0,
    }).sum()
}

/// Files still open (playing, downloading) are skipped.
pub fn clear_dir(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    // Markers last, so a clip is gone before its marker is.
    entries.sort_by_key(|p| is_part_marker(p));
    for path in entries {
        if path.is_dir() {
            clear_dir(&path);
            let _ = std::fs::remove_dir(&path);
        } else {
            remove_cached(&path);
        }
    }
}

/// `<clip>.part` exists while `<clip>` is still downloading.
pub fn part_marker(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn is_part_marker(path: &Path) -> bool {
    path.extension().is_some_and(|x| x == "part")
}

/// A marker is kept while its clip could not be deleted (still open),
/// so the leftover is never taken for a finished clip.
fn remove_cached(path: &Path) {
    if is_part_marker(path) && path.with_extension("").exists() {
        return;
    }
    let _ = std::fs::remove_file(path);
}

pub fn prune_old_runtimes() {
    let keep = [env!("LIBMPV_HASH"), env!("HELPER_HASH")];
    if let Ok(entries) = std::fs::read_dir(data_dir().join("runtime")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !keep.contains(&name.as_str()) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}
