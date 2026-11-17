//! Minimal libmpv binding, loaded at runtime with libloading (no import library needed).

use std::collections::VecDeque;
use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::path::Path;
use std::sync::{Arc, Mutex};

type Handle = *mut c_void;

#[repr(C)]
struct MpvEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventEndFile {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
struct MpvEventLogMessage {
    prefix: *const c_char,
    level: *const c_char,
    text: *const c_char,
    log_level: c_int,
}

const MPV_EVENT_SHUTDOWN: c_int = 1;
const MPV_EVENT_LOG_MESSAGE: c_int = 2;
const LOG_KEEP: usize = 60;
const MPV_EVENT_START_FILE: c_int = 6;
const MPV_EVENT_END_FILE: c_int = 7;
const MPV_EVENT_FILE_LOADED: c_int = 8;
const MPV_END_FILE_REASON_EOF: c_int = 0;
const MPV_END_FILE_REASON_ERROR: c_int = 4;

struct Api {
    create: unsafe extern "C" fn() -> Handle,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    set_option_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    get_property_string: unsafe extern "C" fn(Handle, *const c_char) -> *mut c_char,
    command: unsafe extern "C" fn(Handle, *const *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(Handle, c_double) -> *mut MpvEvent,
    free: unsafe extern "C" fn(*mut c_void),
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    request_log_messages: unsafe extern "C" fn(Handle, *const c_char) -> c_int,
}

/// Last thing mpv reported about the current file, for the UI status line.
#[derive(Clone, Default, serde::Serialize)]
pub struct PlaybackEvent {
    pub state: String,
    pub error: Option<String>,
}

pub struct Mpv {
    _lib: libloading::Library,
    api: Api,
    handle: Handle,
    pub last_event: Arc<Mutex<PlaybackEvent>>,
    /// Recent libmpv/ffmpeg warnings and errors for the current file, passwords removed.
    pub log: Arc<Mutex<VecDeque<String>>>,
}

// The mpv client API is thread-safe: every function may be called from any thread.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

/// Remove `user:password@` from any URL in a log line.
pub fn redact(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str("***@");
                out.push_str(&authority[at + 1..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

impl Mpv {
    /// Load libmpv from `dll`, create a player that draws into the native window `wid`.
    pub fn new(dll: &Path, wid: isize) -> Result<Arc<Self>, String> {
        unsafe {
            let lib = libloading::Library::new(dll)
                .map_err(|e| format!("could not load {}: {e}", dll.display()))?;
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes())
                        .map_err(|e| format!("libmpv is missing {}: {e}", $name))?
                };
            }
            let api = Api {
                create: sym!("mpv_create"),
                initialize: sym!("mpv_initialize"),
                set_option_string: sym!("mpv_set_option_string"),
                set_property_string: sym!("mpv_set_property_string"),
                get_property_string: sym!("mpv_get_property_string"),
                command: sym!("mpv_command"),
                wait_event: sym!("mpv_wait_event"),
                free: sym!("mpv_free"),
                error_string: sym!("mpv_error_string"),
                request_log_messages: sym!("mpv_request_log_messages"),
            };

            let handle = (api.create)();
            if handle.is_null() {
                return Err("mpv_create failed".into());
            }

            let mpv = Mpv {
                _lib: lib,
                api,
                handle,
                last_event: Arc::new(Mutex::new(PlaybackEvent {
                    state: "idle".into(),
                    error: None,
                })),
                log: Arc::new(Mutex::new(VecDeque::new())),
            };

            let wid_s = wid.to_string();
            for (k, v) in [
                ("wid", wid_s.as_str()),
                ("vo", "gpu-next"),
                ("hwdec", "auto-safe"),
                ("idle", "yes"),
                ("keep-open", "yes"),
                ("force-window", "yes"),
                ("osc", "no"),
                ("osd-level", "0"),
                ("input-default-bindings", "no"),
                ("input-vo-keyboard", "no"),
                ("input-cursor", "no"),
                ("cursor-autohide", "no"),
                ("rtsp-transport", "tcp"),
                ("network-timeout", "8"),
                ("background-color", "#000000"),
                ("terminal", "no"),
            ] {
                let r = (mpv.api.set_option_string)(handle, cstr(k).as_ptr(), cstr(v).as_ptr());
                if r < 0 {
                    crate::applog::write(&format!("mpv option {k}={v} rejected: {}", mpv.err(r)));
                }
            }

            let r = (mpv.api.initialize)(handle);
            if r < 0 {
                return Err(format!("mpv_initialize failed: {}", mpv.err(r)));
            }

            (mpv.api.request_log_messages)(handle, cstr("warn").as_ptr());

            let mpv = Arc::new(mpv);
            Self::spawn_event_thread(mpv.clone());
            Ok(mpv)
        }
    }

    fn err(&self, code: c_int) -> String {
        unsafe {
            CStr::from_ptr((self.api.error_string)(code))
                .to_string_lossy()
                .into_owned()
        }
    }

    fn spawn_event_thread(mpv: Arc<Mpv>) {
        std::thread::spawn(move || loop {
            let ev = unsafe { &*(mpv.api.wait_event)(mpv.handle, 1.0) };
            let update = match ev.event_id {
                MPV_EVENT_SHUTDOWN => break,
                MPV_EVENT_LOG_MESSAGE if !ev.data.is_null() => {
                    mpv.record_log(unsafe { &*(ev.data as *const MpvEventLogMessage) });
                    None
                }
                MPV_EVENT_START_FILE => Some(("opening", None)),
                MPV_EVENT_FILE_LOADED => Some(("playing", None)),
                MPV_EVENT_END_FILE if !ev.data.is_null() => {
                    let ef = unsafe { &*(ev.data as *const MpvEventEndFile) };
                    match ef.reason {
                        MPV_END_FILE_REASON_ERROR => Some(("error", Some(mpv.err(ef.error)))),
                        MPV_END_FILE_REASON_EOF => Some(("ended", None)),
                        _ => Some(("stopped", None)),
                    }
                }
                _ => None,
            };
            if let Some((state, error)) = update {
                *mpv.last_event.lock().unwrap() = PlaybackEvent {
                    state: state.into(),
                    error,
                };
            }
        });
    }

    fn record_log(&self, m: &MpvEventLogMessage) {
        let s = |p: *const c_char| unsafe {
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        let line = redact(&format!("[{}] {}: {}", s(m.prefix), s(m.level), s(m.text).trim_end()));
        crate::applog::write(&format!("player: {line}"));
        let mut log = self.log.lock().unwrap();
        if log.len() >= LOG_KEEP {
            log.pop_front();
        }
        log.push_back(line);
    }

    pub fn clear_log(&self) {
        self.log.lock().unwrap().clear();
        *self.last_event.lock().unwrap() = PlaybackEvent { state: "opening".into(), error: None };
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let r = unsafe { (self.api.command)(self.handle, ptrs.as_ptr()) };
        if r < 0 {
            Err(format!("{}: {}", args.first().unwrap_or(&""), self.err(r)))
        } else {
            Ok(())
        }
    }

    pub fn set(&self, name: &str, value: &str) -> Result<(), String> {
        let r = unsafe {
            (self.api.set_property_string)(self.handle, cstr(name).as_ptr(), cstr(value).as_ptr())
        };
        if r < 0 {
            Err(format!("{name}: {}", self.err(r)))
        } else {
            Ok(())
        }
    }

    pub fn get(&self, name: &str) -> Option<String> {
        unsafe {
            let p = (self.api.get_property_string)(self.handle, cstr(name).as_ptr());
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            (self.api.free)(p as *mut c_void);
            Some(s)
        }
    }
}
