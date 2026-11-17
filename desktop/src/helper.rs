//! Talks to the frozen Python helper (pytapo) over stdin/stdout, one JSON object per line.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A helper failure, carried to the UI as a code it can explain.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HelperError {
    pub code: String,
    pub message: String,
}

impl HelperError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, HelperError>>>>>;
type EventSink = Arc<dyn Fn(Value) + Send + Sync>;

struct Proc {
    child: Child,
    stdin: ChildStdin,
}

pub struct Helper {
    proc: Mutex<Option<Proc>>,
    pending: Pending,
    next_id: AtomicU64,
    on_event: EventSink,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

impl Helper {
    pub fn new(on_event: EventSink, log: Arc<dyn Fn(&str) + Send + Sync>) -> Self {
        Self {
            proc: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            on_event,
            log,
        }
    }

    fn spawn(&self) -> Result<Proc, HelperError> {
        let exe = crate::runtime::helper_exe()
            .map_err(|e| HelperError::new("ERR_HELPER_START", e))?;
        let mut child = Command::new(&exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| HelperError::new("ERR_HELPER_START", format!("{}: {e}", exe.display())))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        (self.log)(&format!("camera helper started (pid {})", child.id()));
        let pending = self.pending.clone();
        let on_event = self.on_event.clone();
        let log_exit = self.log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                if msg.get("event").is_some() {
                    on_event(msg);
                    continue;
                }
                let Some(id) = msg.get("id").and_then(Value::as_u64) else { continue };
                let reply = if msg.get("ok").and_then(Value::as_bool) == Some(true) {
                    Ok(msg.get("result").cloned().unwrap_or(Value::Null))
                } else {
                    Err(HelperError::new(
                        msg.get("code").and_then(Value::as_str).unwrap_or("ERR_UNKNOWN"),
                        msg.get("message").and_then(Value::as_str).unwrap_or(""),
                    ))
                };
                if let Some(tx) = pending.lock().unwrap().remove(&id) {
                    let _ = tx.send(reply);
                }
            }
            log_exit("camera helper stopped");
            // Helper exited: fail everything still waiting.
            for (_, tx) in pending.lock().unwrap().drain() {
                let _ = tx.send(Err(HelperError::new(
                    "ERR_HELPER_STOPPED",
                    "The camera helper stopped unexpectedly.",
                )));
            }
        });

        let log = self.log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                log(&format!("helper: {line}"));
            }
        });
        Ok(Proc { child, stdin })
    }

    /// Start the helper now so the first real request does not wait for it.
    pub fn warm_up(&self) {
        if let Err(e) = self.ensure() {
            (self.log)(&format!("camera helper could not start: {} {}", e.code, e.message));
        }
    }

    fn ensure(&self) -> Result<(), HelperError> {
        let mut guard = self.proc.lock().unwrap();
        let alive = match guard.as_mut() {
            Some(p) => matches!(p.child.try_wait(), Ok(None)),
            None => false,
        };
        if !alive {
            *guard = Some(self.spawn()?);
        }
        Ok(())
    }

    /// Send one command and wait for its answer.
    pub async fn call(&self, cmd: &str, mut args: Value, timeout: Duration) -> Result<Value, HelperError> {
        self.ensure()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);

        if !args.is_object() {
            args = json!({});
        }
        args["id"] = json!(id);
        args["cmd"] = json!(cmd);
        let line = format!("{args}\n");
        {
            let mut guard = self.proc.lock().unwrap();
            let p = guard.as_mut().ok_or_else(|| HelperError::new("ERR_HELPER_STOPPED", "helper not running"))?;
            if let Err(e) = p.stdin.write_all(line.as_bytes()).and_then(|_| p.stdin.flush()) {
                self.pending.lock().unwrap().remove(&id);
                *guard = None;
                return Err(HelperError::new("ERR_HELPER_STOPPED", e.to_string()));
            }
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(HelperError::new("ERR_HELPER_STOPPED", "no answer from helper")),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(HelperError::new("ERR_CAMERA_TIMEOUT", format!("{cmd} took longer than {}s", timeout.as_secs())))
            }
        }
    }

    /// Fire-and-forget (used for cancel, which must not queue behind a download).
    pub fn send(&self, cmd: &str) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Some(p) = self.proc.lock().unwrap().as_mut() {
            let _ = writeln!(p.stdin, "{}", json!({"id": id, "cmd": cmd}));
            let _ = p.stdin.flush();
        }
    }

    pub fn shutdown(&self) {
        if let Some(mut p) = self.proc.lock().unwrap().take() {
            let _ = writeln!(p.stdin, "{}", json!({"id": 0, "cmd": "shutdown"}));
            let _ = p.stdin.flush();
            std::thread::sleep(Duration::from_millis(150));
            let _ = p.child.kill();
        }
    }
}
