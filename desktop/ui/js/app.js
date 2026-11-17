import { initI18n, setLanguage, t, apply, languageList, languageCode, intlLocale } from "./i18n.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const appWindow = window.__TAURI__.window.getCurrentWindow();
const dialog = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
};

const state = {
  view: "boot",
  connected: false,
  device: null,
  quality: "HD",
  dates: new Set(),
  month: new Date(),
  selected: null,
  clips: [],
  filter: "all",
  downloads: [],
  pendingPlay: null,
  status: { ready: false },
  lastPlayerError: null,
  seeking: false,
  dialogOpen: false,
  problems: [],
  // clip start -> data URL, null if the camera has none
  thumbs: new Map(),
};

// ---------------------------------------------------------------- helpers
const pad = (n) => String(n).padStart(2, "0");
const isoDate = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const fromIso = (s) => {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
};
const fmtClock = (unix) => {
  const d = new Date(unix * 1000);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};
const fmtLen = (sec) => {
  sec = Math.max(0, Math.round(sec || 0));
  const h = Math.floor(sec / 3600), m = Math.floor((sec % 3600) / 60), s = sec % 60;
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
};
const fmtDay = (d, opts = { weekday: "long", day: "numeric", month: "long" }) =>
  new Intl.DateTimeFormat(intlLocale(), opts).format(d);
const fileName = (p) => (p || "").split(/[\\/]/).pop();

function errorText(code) {
  const known = t(`errors.${code}.title`) !== `errors.${code}.title`;
  const key = known ? code : "ERR_UNKNOWN";
  const steps = t(`errors.${key}.steps`);
  return {
    code,
    title: t(`errors.${key}.title`),
    explanation: t(`errors.${key}.explanation`),
    steps: Array.isArray(steps) ? steps : [],
  };
}

const asError = (e) => (e && typeof e === "object" && e.code ? e : { code: "ERR_UNKNOWN", message: String(e) });

// ---------------------------------------------------------------- problems
function loadProblems() {
  try {
    state.problems = JSON.parse(localStorage.getItem("problems") || "[]");
  } catch {
    state.problems = [];
  }
}

function addProblem(code, technical, context) {
  const entry = { code, technical: technical || "", context, at: Date.now() };
  state.problems.unshift(entry);
  state.problems = state.problems.slice(0, 50);
  localStorage.setItem("problems", JSON.stringify(state.problems));
  invoke("log_line", { line: `problem ${code} (${context}): ${technical || ""}` }).catch(() => {});
  localStorage.setItem("problemsSeen", localStorage.getItem("problemsSeen") || "0");
  renderProblemCount();
  if (state.view === "problems") renderProblems();
}

function renderProblemCount() {
  const seen = Number(localStorage.getItem("problemsSeen") || 0);
  const unseen = state.problems.filter((p) => p.at > seen).length;
  const badge = $("problemCount");
  badge.hidden = unseen === 0;
  badge.textContent = unseen;
}

function renderProblems() {
  localStorage.setItem("problemsSeen", String(Date.now()));
  renderProblemCount();
  const list = $("problemList");
  list.replaceChildren();
  $("problemsEmpty").hidden = state.problems.length > 0;
  $("btnClearProblems").disabled = state.problems.length === 0;
  for (const p of state.problems) {
    const info = errorText(p.code);
    const li = el("li", "problem");
    const when = new Date(p.at);
    li.append(el("p", "problem-when", new Intl.DateTimeFormat(intlLocale(), { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(when)));
    const body = el("div", "problem-body");
    body.append(el("h2", null, info.title), el("p", null, info.explanation));
    if (info.steps.length) {
      body.append(el("p", "problem-label", t("problems.whatToDo")));
      const ol = el("ol", "steps");
      info.steps.forEach((s) => ol.append(el("li", null, s)));
      body.append(ol);
    }
    const details = el("details", "tech");
    const tech = `${t("problems.code", { code: p.code })}\n${when.toISOString()}\n${p.context}\n${p.technical}`.trim();
    details.append(el("summary", null, t("problems.technical")), el("pre", null, tech));
    const copy = el("button", "btn btn--quiet", t("problems.copy"));
    copy.type = "button";
    copy.addEventListener("click", async () => {
      await navigator.clipboard.writeText(tech).catch(() => {});
      copy.textContent = t("problems.copied");
      setTimeout(() => (copy.textContent = t("problems.copy")), 1500);
    });
    details.append(copy);
    body.append(details);
    li.append(body);
    list.append(li);
  }
}

// ---------------------------------------------------------------- toasts
function toast(text, kind) {
  const n = el("div", `toast${kind ? ` toast--${kind}` : ""}`, text);
  $("toasts").append(n);
  setTimeout(() => n.remove(), 4500);
}

// ---------------------------------------------------------------- views
const VIEWS = ["login", "camera", "downloads", "problems", "settings", "about"];
function setView(view) {
  if (!VIEWS.includes(view)) view = state.connected ? "camera" : "login";
  if (!state.connected && (view === "camera" || view === "downloads")) view = "login";
  if (state.connected && view === "login") view = "camera";
  state.view = view;
  document.body.dataset.view = view;
  document.querySelectorAll("[data-view-link]").forEach((a) => {
    if (a.dataset.viewLink === view) a.setAttribute("aria-current", "page");
    else a.removeAttribute("aria-current");
  });
  if (location.hash.slice(1) !== view && view !== "boot") history.replaceState(null, "", `#${view}`);
  if (view === "problems") renderProblems();
  if (view === "settings") renderSettings();
  if (view === "about") renderAbout();
  if (view === "downloads") renderDownloads();
  if (view === "camera") requestAnimationFrame(syncRect);
  updateVideoVisibility();
}

function showShell(connected) {
  document.querySelectorAll("[data-auth]").forEach((a) => {
    a.hidden = a.dataset.auth === (connected ? "out" : "in");
  });
  $("btnSignOut").hidden = !connected;
  $("deviceName").textContent = connected && state.device ? deviceLabel() : "";
}

function deviceLabel() {
  const d = state.device;
  const name = d.alias || d.model || "";
  return name ? `${name}, ${d.host}` : t("camera.host", { host: d.host });
}

window.addEventListener("hashchange", () => {
  const v = location.hash.slice(1);
  if (v && v !== state.view) setView(v);
});
document.querySelectorAll("[data-view-link]").forEach((a) =>
  a.addEventListener("click", (e) => {
    e.preventDefault();
    setView(a.dataset.viewLink);
  }),
);

// ---------------------------------------------------------------- video window
let lastRect = "";
function syncRect() {
  if (state.view !== "camera") return;
  const box = $("video").getBoundingClientRect();
  const s = window.devicePixelRatio || 1;
  const inset = 2; // keep the 2px frame visible around the picture
  const rect = {
    x: Math.round((box.left + inset) * s),
    y: Math.round((box.top + inset) * s),
    w: Math.max(1, Math.round((box.width - inset * 2) * s)),
    h: Math.max(1, Math.round((box.height - inset * 2) * s)),
  };
  const key = JSON.stringify(rect);
  if (key === lastRect) return;
  lastRect = key;
  invoke("set_video_rect", rect).catch((e) => {
    const err = asError(e);
    if (state.lastPlayerError !== err.code) {
      state.lastPlayerError = err.code;
      addProblem(err.code, err.message, "player");
    }
  });
}
new ResizeObserver(syncRect).observe($("video"));
window.addEventListener("resize", syncRect);

let videoShown = null;
function updateVideoVisibility() {
  const st = state.status;
  const show =
    state.view === "camera" && !state.dialogOpen && st.ready && st.mode !== "none" && st.state === "playing" && !st.error;
  $("video").classList.toggle("is-playing", show);
  if (show !== videoShown) {
    videoShown = show;
    invoke("set_video_visible", { visible: show }).catch(() => {});
    if (show) {
      lastRect = "";
      syncRect();
    }
  }
}

// ---------------------------------------------------------------- sign in
async function fillSavedLogin() {
  const s = await invoke("saved_login").catch(() => ({}));
  $("host").value = s.host || "";
  $("username").value = s.username || "";
  $("password").value = "";
  $("cloudPassword").value = "";
  state.savedPassword = !!s.hasPassword;
  $("password").placeholder = s.hasPassword ? t("login.savedPassword") : "";
  $("cloudPassword").placeholder = s.hasCloudPassword ? t("login.savedPassword") : "";
  $("remember").checked = !!s.remember;
  if (s.hasCloudPassword) $("cloudToggle").checked = true;
  $("cloudField").hidden = !$("cloudToggle").checked;
}

$("cloudToggle").addEventListener("change", () => {
  $("cloudField").hidden = !$("cloudToggle").checked;
});
document.querySelectorAll("[data-reveal]").forEach((b) =>
  b.addEventListener("click", () => {
    const input = $(b.dataset.reveal);
    const show = input.type === "password";
    input.type = show ? "text" : "password";
    b.textContent = t(show ? "login.hide" : "login.show");
  }),
);

function hideLoginError() {
  $("loginError").hidden = true;
  $("loginHelp").hidden = false;
}

function showLoginError(err) {
  $("loginHelp").hidden = true;
  const info = errorText(err.code);
  $("loginErrorTitle").textContent = info.title;
  $("loginErrorText").textContent = info.explanation;
  const ol = $("loginErrorSteps");
  ol.replaceChildren(...info.steps.map((s) => el("li", null, s)));
  $("loginError").hidden = false;
}

$("loginForm").addEventListener("submit", async (e) => {
  e.preventDefault();
  const host = $("host").value.trim();
  const username = $("username").value.trim();
  const password = $("password").value;
  for (const id of ["host", "username", "password"]) $(id).removeAttribute("aria-invalid");
  const missing = [["host", host], ["username", username], ["password", password || (state.savedPassword ? "saved" : "")]].filter(([, v]) => !v);
  if (missing.length) {
    missing.forEach(([id]) => $(id).setAttribute("aria-invalid", "true"));
    showLoginError({ code: "ERR_INPUT" });
    $(missing[0][0]).focus();
    return;
  }
  const cloud = $("cloudToggle").checked ? $("cloudPassword").value : "";
  const btn = $("btnConnect");
  btn.classList.add("is-loading");
  btn.disabled = true;
  btn.querySelector("[data-i18n]").textContent = t("login.connecting");
  hideLoginError();
  const stage = $("connectStage");
  stage.textContent = t("login.checking");
  const stageTimer = setTimeout(() => (stage.textContent = t("login.signingIn")), 2600);
  try {
    const info = await invoke("connect", {
      host, username, password, cloudPassword: $("cloudToggle").checked ? cloud : null, remember: $("remember").checked,
    });
    state.connected = true;
    state.device = { ...info, host };
    // Do not keep typed passwords in the page once signed in.
    $("password").value = "";
    $("cloudPassword").value = "";
    if (info.warning) addProblem(info.warning, "", "sign-in");
    showShell(true);
    setView("camera");
    startCamera();
  } catch (e) {
    const err = asError(e);
    addProblem(err.code, err.message, "sign-in");
    showLoginError(err);
  } finally {
    clearTimeout(stageTimer);
    stage.textContent = "";
    btn.classList.remove("is-loading");
    btn.disabled = false;
    btn.querySelector("[data-i18n]").textContent = t("login.connect");
  }
});

$("btnTest").addEventListener("click", async () => {
  const btn = $("btnTest");
  const out = $("testResult");
  const host = $("host").value.trim();
  out.classList.remove("is-ok", "is-bad");
  if (!host) {
    $("host").setAttribute("aria-invalid", "true");
    $("host").focus();
    return;
  }
  btn.classList.add("is-loading");
  btn.disabled = true;
  out.textContent = t("login.testing");
  try {
    const r = await invoke("test_camera", { host });
    const msg = r.control && r.video ? "testOk" : r.control ? "testNoVideo" : r.video ? "testNoControl" : "testNone";
    out.textContent = t(`login.${msg}`);
    out.classList.add(msg === "testOk" ? "is-ok" : "is-bad");
    $("host").toggleAttribute("aria-invalid", msg === "testNone");
  } catch (e) {
    const err = asError(e);
    out.textContent = t(err.code === "ERR_BAD_HOST" ? "login.testBadHost" : "login.testNone");
    out.classList.add("is-bad");
  } finally {
    btn.classList.remove("is-loading");
    btn.disabled = false;
  }
});
for (const id of ["host", "username"]) {
  $(id).addEventListener("change", () => {
    // A saved password belongs to one camera and user only.
    state.savedPassword = false;
    $("password").placeholder = "";
    $("cloudPassword").placeholder = "";
  });
}
$("host").addEventListener("input", () => {
  $("testResult").textContent = "";
  $("testResult").classList.remove("is-ok", "is-bad");
  $("host").removeAttribute("aria-invalid");
});

$("btnSignOut").addEventListener("click", async () => {
  await invoke("disconnect").catch(() => {});
  state.connected = false;
  state.device = null;
  state.clips = [];
  state.dates = new Set();
  state.thumbs.clear();
  showShell(false);
  await fillSavedLogin();
  setView("login");
});

// ---------------------------------------------------------------- camera
// ---------------------------------------------------------------- pan / tilt
// Shown only when the camera reports a motor; each press is one small step and
// the camera's firmware stops at its own limits.
const STEP = 10;
const MOVES = { left: [-STEP, 0], right: [STEP, 0], up: [0, STEP], down: [0, -STEP] };
state.ptz = { panTilt: false, presets: [] };
let moving = false;

async function loadPtz() {
  state.ptz = { panTilt: false, presets: [] };
  try {
    state.ptz = await invoke("ptz_info");
  } catch (e) {
    const err = asError(e);
    invoke("log_line", { line: `pan/tilt check failed: ${err.code} ${err.message}` }).catch(() => {});
  }
  renderPtz();
}

function renderPtz() {
  const live = state.status.mode === "live";
  $("ptz").hidden = !(state.ptz.panTilt && live);
  const sel = $("presetSelect");
  const presets = state.ptz.presets || [];
  sel.hidden = !(state.ptz.panTilt && live && presets.length);
  if (sel.options.length !== presets.length + 1) {
    const first = el("option", null, t("ptz.presets"));
    first.value = "";
    sel.replaceChildren(first, ...presets.map((p) => {
      const o = el("option", null, p.name);
      o.value = p.id;
      return o;
    }));
  }
}

async function ptzCall(cmd, args) {
  if (moving) return;
  moving = true;
  $("ptz").classList.add("is-moving");
  $("ptz").querySelectorAll("button").forEach((b) => (b.disabled = true));
  $("presetSelect").disabled = true;
  try {
    await invoke(cmd, args);
  } catch (e) {
    const err = asError(e);
    if (err.code === "ERR_PTZ_MOVE") toast(t("ptz.edge"));
    else if (err.code !== "ERR_PTZ_BUSY") {
      addProblem(err.code, err.message, "pan and tilt");
      toast(errorText(err.code).title, "error");
    }
  } finally {
    moving = false;
    $("ptz").classList.remove("is-moving");
    $("ptz").querySelectorAll("button").forEach((b) => (b.disabled = false));
    $("presetSelect").disabled = false;
  }
}

$("ptz").querySelectorAll("[data-move]").forEach((b) =>
  b.addEventListener("click", () => {
    const [dx, dy] = MOVES[b.dataset.move];
    ptzCall("ptz_move", { dx, dy });
  }),
);
$("presetSelect").addEventListener("change", (e) => {
  const id = e.target.value;
  e.target.value = "";
  if (id) ptzCall("ptz_goto", { id });
});
window.addEventListener("keydown", (e) => {
  if (state.view !== "camera" || $("ptz").hidden || e.target.closest("input, select, textarea")) return;
  const key = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" }[e.key];
  if (!key || e.target.closest(".days")) return;
  e.preventDefault();
  const [dx, dy] = MOVES[key];
  ptzCall("ptz_move", { dx, dy });
});

async function startCamera() {
  const today = new Date();
  state.month = new Date(today.getFullYear(), today.getMonth(), 1);
  state.selected = isoDate(today);
  renderCalendar();
  loadDates();
  loadClips();
  loadPtz();
  await waitForPlayer();
  startLive();
}

async function waitForPlayer() {
  for (let i = 0; i < 100 && !state.status.ready && !state.status.failed; i++) {
    await new Promise((r) => setTimeout(r, 100));
  }
}

async function startLive() {
  try {
    await invoke("start_live", { quality: state.quality });
  } catch (e) {
    const err = asError(e);
    addProblem(err.code, err.message, "live view");
  }
}

$("btnStartLive").addEventListener("click", () => {
  const st = state.status;
  if (st.mode === "clip" && st.error && st.path) invoke("play_file", { path: st.path }).catch(() => {});
  else startLive();
});
$("btnBackLive").addEventListener("click", startLive);

document.querySelectorAll("#quality [data-quality]").forEach((b) =>
  b.addEventListener("click", () => {
    state.quality = b.dataset.quality;
    renderQuality();
    startLive();
  }),
);
function renderQuality() {
  document.querySelectorAll("#quality [data-quality]").forEach((b) =>
    b.setAttribute("aria-checked", String(b.dataset.quality === state.quality)),
  );
}

$("btnPlayPause").addEventListener("click", () => {
  if (state.status.mode === "none") startLive();
  else invoke("player_control", { action: "pause" }).catch(() => {});
});
$("btnStop").addEventListener("click", () => invoke("player_control", { action: "stop" }).catch(() => {}));
$("btnBack10").addEventListener("click", () => invoke("player_control", { action: "seek", value: "-10" }).catch(() => {}));
$("btnFwd10").addEventListener("click", () => invoke("player_control", { action: "seek", value: "10" }).catch(() => {}));
$("btnMute").addEventListener("click", () => invoke("player_control", { action: "mute" }).catch(() => {}));
$("volume").addEventListener("input", (e) => invoke("player_control", { action: "volume", value: e.target.value }).catch(() => {}));
$("seek").addEventListener("input", (e) => {
  state.seeking = true;
  // mpv cannot seek past what is on disk yet.
  const part = downloadedPart(state.status);
  const max = Math.max(0, Math.floor(part * 1000) - 20);
  if (part < 1 && e.target.value > max) e.target.value = max;
});
$("seek").addEventListener("change", (e) => {
  const d = clipLength(state.status);
  invoke("player_control", { action: "seek_to", value: String((e.target.value / 1000) * d) }).catch(() => {});
  state.seeking = false;
});

function downloadedPart(st) {
  const d = state.downloads.find((x) => x.path === st.path && x.state === "downloading");
  return d ? d.progress : 1;
}

async function setFullscreen(on) {
  await appWindow.setFullscreen(on).catch(() => {});
  document.body.classList.toggle("is-fullscreen", on);
  $("btnFullscreen").setAttribute("aria-label", t(on ? "camera.exitFullscreen" : "camera.fullscreen"));
  $("btnFullscreen").title = t(on ? "camera.exitFullscreen" : "camera.fullscreen");
  lastRect = "";
  requestAnimationFrame(() => requestAnimationFrame(syncRect));
}
$("btnExternal").addEventListener("click", () => {
  const path = state.status.path;
  if (!path) return;
  invoke("player_control", { action: "pause" }).catch(() => {});
  invoke("open_external", { path }).catch((e) => toast(errorText(asError(e).code).title, "error"));
});
$("btnSnapshot").addEventListener("click", async () => {
  try {
    const path = await invoke("take_screenshot");
    toast(t("camera.screenshotSaved", { name: fileName(path) }));
  } catch (e) {
    const err = asError(e);
    addProblem(err.code, err.message, "screenshot");
    toast(errorText(err.code).title, "error");
  }
});
$("btnFullscreen").addEventListener("click", () => setFullscreen(!document.body.classList.contains("is-fullscreen")));
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && document.body.classList.contains("is-fullscreen")) setFullscreen(false);
  if (state.view === "camera" && e.key === " " && !e.target.closest("input, button, select, textarea")) {
    e.preventDefault();
    $("btnPlayPause").click();
  }
});
$("video").addEventListener("dblclick", () => setFullscreen(!document.body.classList.contains("is-fullscreen")));

function playingClip(st) {
  if (st.mode !== "clip" || !st.path) return null;
  return state.clips.find((x) => x.file === st.path) || state.downloads.find((d) => d.path === st.path) || null;
}

// A growing file has no final length yet.
function clipLength(st) {
  const c = playingClip(st);
  return c ? c.end - c.start : st.duration || 0;
}

function renderPlayer(st) {
  const controls = $("controls");
  const live = st.mode === "live";
  const clip = st.mode === "clip";
  controls.classList.toggle("is-clip", clip);
  controls.classList.toggle("is-paused", !!st.paused || st.mode === "none");
  controls.classList.toggle("is-muted", !!st.muted);
  const pp = $("btnPlayPause");
  const ppLabel = t(st.paused || st.mode === "none" ? "camera.play" : "camera.pause");
  pp.setAttribute("aria-label", ppLabel);
  pp.title = ppLabel;
  const muteLabel = t(st.muted ? "camera.unmute" : "camera.mute");
  $("btnMute").setAttribute("aria-label", muteLabel);
  $("btnMute").title = muteLabel;
  for (const id of ["btnPlayPause", "btnStop", "btnMute", "volume", "btnFullscreen"]) $(id).disabled = !st.ready;
  $("btnStop").disabled = !st.ready || st.mode === "none";
  if (st.ready && document.activeElement !== $("volume") && st.volume != null) $("volume").value = Math.round(st.volume);

  $("livePlate").hidden = !(live && st.state === "playing" && !st.error);
  $("btnBackLive").hidden = !clip;
  const tip = $("clipTip");
  if (tip.hidden === clip) {
    tip.hidden = !clip;
    $("quality").hidden = clip;
    lastRect = "";
    requestAnimationFrame(syncRect);
  }
  renderPtz();
  $("btnSnapshot").disabled = !st.ready || st.mode === "none" || st.state !== "playing";
  if (clip && st.path) {
    const c = playingClip(st);
    $("playerTitle").textContent = c
      ? t("camera.clipTitle", { date: fmtDay(new Date(c.start * 1000), { day: "numeric", month: "short" }), time: fmtClock(c.start) })
      : fileName(st.path);
  } else {
    $("playerTitle").textContent = t("camera.liveTitle");
  }

  $("timeNow").textContent = live ? "" : fmtLen(st.position);
  const length = clipLength(st);
  $("timeTotal").textContent = fmtLen(length);
  if (clip && !state.seeking && length) $("seek").value = Math.round(((st.position || 0) / length) * 1000);
  $("seek").style.setProperty("--played", `${$("seek").value / 10}%`);
  $("seek").style.setProperty("--loaded", `${downloadedPart(st) * 100}%`);

  // Message shown in the video box whenever the picture is not
  const msg = $("videoMsgText");
  const detail = $("videoMsgDetail");
  const btn = $("btnStartLive");
  detail.hidden = true;
  btn.hidden = false;
  btn.textContent = t("camera.startLive");
  if (!st.ready) {
    if (st.failed) {
      const info = errorText("ERR_PLAYER_START");
      msg.textContent = info.title;
      detail.textContent = info.explanation;
      detail.hidden = false;
    } else msg.textContent = t("camera.playerStarting");
    btn.hidden = true;
  } else if (st.error) {
    const info = errorText(st.error);
    msg.textContent = info.title;
    detail.textContent = info.explanation;
    detail.hidden = false;
    btn.textContent = t("camera.retry");
  } else if (st.mode === "none") {
    msg.textContent = t(st.livePaused ? "camera.livePaused" : "camera.stopped");
  } else if (st.state !== "playing") {
    msg.textContent = t("camera.opening");
    btn.hidden = true;
  }

  if (st.error && st.error !== state.lastPlayerError) {
    addProblem(st.error, st.detail, live ? "live view" : "playback");
  }
  state.lastPlayerError = st.error || null;
}

// The player is only asked for its state while signed in: 400 ms on the camera
// page, slower elsewhere, so the app sits idle on the other screens.
async function pollPlayer() {
  if (state.connected) {
    try {
      state.status = await invoke("player_status");
      renderPlayer(state.status);
    } catch {
      /* keep polling */
    }
    updateVideoVisibility();
  }
  setTimeout(pollPlayer, !state.connected ? 1000 : state.view === "camera" ? 400 : 2000);
}

// ---------------------------------------------------------------- calendar
function renderCalendar() {
  const first = state.month;
  $("monthName").textContent = new Intl.DateTimeFormat(intlLocale(), { month: "long", year: "numeric" }).format(first);
  const weekStart = Number(t("calendar.weekStart")) || 0;
  const names = t("calendar.weekdays");
  const wd = $("weekdays");
  wd.replaceChildren();
  for (let i = 0; i < 7; i++) {
    // names are listed Monday first
    const idx = (weekStart + i + 6) % 7;
    wd.append(el("span", null, Array.isArray(names) ? names[idx] : ""));
  }
  const days = $("days");
  days.replaceChildren();
  const offset = (first.getDay() - weekStart + 7) % 7;
  const start = new Date(first.getFullYear(), first.getMonth(), 1 - offset);
  const lastOfMonth = new Date(first.getFullYear(), first.getMonth() + 1, 0);
  const cells = Math.ceil((offset + lastOfMonth.getDate()) / 7) * 7;
  const today = isoDate(new Date());
  const tpl = $("tplDay");
  for (let i = 0; i < cells; i++) {
    const d = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i);
    const iso = isoDate(d);
    const b = tpl.content.firstElementChild.cloneNode(true);
    b.textContent = d.getDate();
    b.dataset.date = iso;
    const has = state.dates.has(iso);
    b.classList.toggle("is-other", d.getMonth() !== first.getMonth());
    b.classList.toggle("has-rec", has);
    if (iso === today) b.setAttribute("aria-current", "date");
    b.setAttribute("aria-selected", String(iso === state.selected));
    b.tabIndex = iso === state.selected ? 0 : -1;
    b.setAttribute("aria-label", t(has ? "calendar.dayLabelRec" : "calendar.dayLabel", { date: fmtDay(d, { weekday: "long", day: "numeric", month: "long", year: "numeric" }) }));
    b.addEventListener("click", () => selectDate(iso));
    days.append(b);
  }
  const latest = new Date(new Date().getFullYear(), new Date().getMonth(), 1);
  $("btnNextMonth").disabled = first >= latest;
}

function selectDate(iso) {
  state.selected = iso;
  const d = fromIso(iso);
  if (d.getMonth() !== state.month.getMonth() || d.getFullYear() !== state.month.getFullYear()) {
    state.month = new Date(d.getFullYear(), d.getMonth(), 1);
  }
  renderCalendar();
  $("days").querySelector(`[data-date="${iso}"]`)?.focus();
  loadClips();
}

$("days").addEventListener("keydown", (e) => {
  const moves = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 };
  if (!(e.key in moves) || !state.selected) return;
  e.preventDefault();
  const d = fromIso(state.selected);
  d.setDate(d.getDate() + moves[e.key]);
  if (d > new Date()) return;
  selectDate(isoDate(d));
});
$("btnPrevMonth").addEventListener("click", () => {
  state.month = new Date(state.month.getFullYear(), state.month.getMonth() - 1, 1);
  renderCalendar();
});
$("btnNextMonth").addEventListener("click", () => {
  state.month = new Date(state.month.getFullYear(), state.month.getMonth() + 1, 1);
  renderCalendar();
});
$("btnRefresh").addEventListener("click", () => {
  loadDates();
  loadClips();
});

async function loadDates() {
  const calState = $("calState");
  calState.hidden = false;
  calState.classList.remove("is-error");
  calState.textContent = t("calendar.loading");
  $("btnRefresh").disabled = true;
  try {
    const dates = await invoke("recording_dates");
    state.dates = new Set(dates);
    calState.hidden = true;
  } catch (e) {
    const err = asError(e);
    addProblem(err.code, err.message, "recording days");
    calState.classList.add("is-error");
    calState.textContent = errorText(err.code).title;
  } finally {
    $("btnRefresh").disabled = false;
    renderCalendar();
  }
}

// ---------------------------------------------------------------- the board
let clipsRequest = 0;
async function loadClips() {
  const req = ++clipsRequest;
  const date = state.selected;
  $("clipsTitle").textContent = fmtDay(fromIso(date));
  $("clipsCount").textContent = "";
  setClipsState(null);
  const list = $("clipList");
  list.replaceChildren(...Array.from({ length: 5 }, () => el("li", "skeleton")));
  list.setAttribute("aria-busy", "true");
  try {
    const clips = await invoke("list_recordings", { date });
    if (req !== clipsRequest) return;
    state.clips = clips;
    renderClips();
  } catch (e) {
    if (req !== clipsRequest) return;
    const err = asError(e);
    addProblem(err.code, err.message, "recordings");
    state.clips = [];
    list.replaceChildren();
    const info = errorText(err.code);
    setClipsState(info.title, info.explanation, true);
  } finally {
    list.removeAttribute("aria-busy");
  }
}

function setClipsState(text, hint, isError) {
  const box = $("clipsState");
  box.hidden = !text;
  box.classList.toggle("is-error", !!isError);
  $("clipsStateText").textContent = text || "";
  $("clipsStateHint").textContent = hint || "";
  $("btnClipsRetry").hidden = !isError;
}
$("btnClipsRetry").addEventListener("click", loadClips);

document.querySelectorAll("#clipFilter [data-filter]").forEach((b) =>
  b.addEventListener("click", () => {
    state.filter = b.dataset.filter;
    document.querySelectorAll("#clipFilter [data-filter]").forEach((x) => x.setAttribute("aria-checked", String(x === b)));
    renderClips();
  }),
);

function clipDownload(c) {
  return state.downloads.filter((d) => d.key === c.key).pop() || null;
}

function clipKind(c) {
  if (!c.types?.length) return t(c.motion ? "clips.motion" : "clips.continuous");
  return c.types.map((n) => {
    const name = t(`clips.types.${n}`);
    return name === `clips.types.${n}` ? t("clips.detection") : name;
  }).filter((v, i, all) => all.indexOf(v) === i).join(", ");
}

// ---------------------------------------------------------------- thumbnails
// Only for clips in view, a few per call.
const thumbWanted = new Set();
let thumbBusy = false;
const thumbObserver = new IntersectionObserver((entries) => {
  for (const e of entries) {
    if (!e.isIntersecting) continue;
    thumbWanted.add(Number(e.target.dataset.start));
    thumbObserver.unobserve(e.target);
  }
  loadThumbs();
}, { root: $("clipList"), rootMargin: "200px" });

async function loadThumbs() {
  // The camera serves one recording stream at a time.
  const downloading = state.downloads.some((d) => d.state === "downloading");
  if (thumbBusy || !thumbWanted.size || !state.connected || downloading) return;
  thumbBusy = true;
  const starts = [...thumbWanted].slice(0, 8);
  starts.forEach((s) => thumbWanted.delete(s));
  try {
    const got = await invoke("clip_thumbnails", { starts });
    for (const s of starts) {
      // Missing key: camera busy, asked again on the next render.
      if (!(s in got)) continue;
      state.thumbs.set(s, got[s]);
      document.querySelectorAll(`img.clip-thumb[data-start="${s}"]`).forEach((img) => {
        if (got[s]) img.src = got[s];
      });
    }
  } catch (e) {
    const err = asError(e);
    invoke("log_line", { line: `thumbnails failed: ${err.code} ${err.message}` }).catch(() => {});
  } finally {
    thumbBusy = false;
  }
  loadThumbs();
}

function clipThumb(c) {
  // The camera keeps an image only for detection recordings.
  if (!c.motion) return el("span", "clip-thumb");
  const img = el("img", "clip-thumb");
  img.alt = "";
  img.dataset.start = c.start;
  const url = state.thumbs.get(c.start);
  if (url) img.src = url;
  else if (url === undefined) thumbObserver.observe(img);
  img.addEventListener("click", () => playClip(c));
  return img;
}

function renderClips() {
  thumbObserver.disconnect();
  const list = $("clipList");
  const shown = state.clips.filter((c) => state.filter === "all" || c.motion);
  const n = state.clips.length;
  $("clipsCount").textContent = n ? (n === 1 ? t("clips.countOne") : t("clips.count", { n })) : "";
  if (!n) setClipsState(t("clips.empty"), t("clips.emptyHint"));
  else if (!shown.length) setClipsState(t("clips.emptyFilter"));
  else setClipsState(null);

  const current = state.status.mode === "clip" ? state.status.path : null;
  list.replaceChildren(
    ...shown.map((c) => {
      const li = el("li", "clip");
      if (current && (c.file || clipDownload(c)?.path) === current) li.classList.add("is-current");
      const start = fmtClock(c.start);
      li.append(clipThumb(c), el("span", "clip-time", start));

      const meta = el("div", "clip-meta");
      meta.append(
        el("span", `kind${c.motion ? " kind--motion" : ""}`, clipKind(c)),
        el("span", "num", t("clips.until", { time: fmtClock(c.end), length: fmtLen(c.end - c.start) })),
      );
      li.append(meta);

      const actions = el("div", "clip-actions");
      const dl = clipDownload(c);
      const busy = dl && (dl.state === "queued" || dl.state === "downloading");
      if (busy) {
        actions.append(el("span", "clip-state num", dl.state === "queued" ? t("clips.queued") : t("clips.downloading", { percent: Math.round(dl.progress * 100) })));
      } else if (dl && dl.state === "failed") {
        actions.append(el("span", "clip-state clip-state--failed", t("clips.failed")));
      } else if (c.file) {
        actions.append(el("span", "clip-state clip-state--done", t("clips.downloaded")));
      }
      const play = el("button", "btn btn--outline", t("clips.play"));
      play.type = "button";
      play.setAttribute("aria-label", t("clips.playClip", { time: start }));
      play.addEventListener("click", () => playClip(c));
      actions.append(play);
      if (!c.file && !busy) {
        const get = el("button", "btn btn--quiet", t(dl && dl.state === "failed" ? "clips.retry" : "clips.download"));
        get.type = "button";
        get.setAttribute("aria-label", t("clips.downloadClip", { time: start }));
        get.addEventListener("click", () => queueDownload(c));
        actions.append(get);
      }
      li.append(actions);
      return li;
    }),
  );
}

async function queueDownload(c, first = false) {
  try {
    await invoke("queue_download", { start: c.start, end: c.end, first });
  } catch (e) {
    const err = asError(e);
    addProblem(err.code, err.message, "download");
    toast(errorText(err.code).title, "error");
  }
}

async function playClip(c) {
  if (c.file) {
    try {
      await invoke("play_file", { path: c.file });
    } catch (e) {
      const err = asError(e);
      addProblem(err.code, err.message, "playback");
      toast(errorText(err.code).title, "error");
    }
    return;
  }
  state.pendingPlay = c.key;
  toast(t("clips.downloadToPlay"));
  queueDownload(c, true);
}

// ---------------------------------------------------------------- downloads
function onDownloads(list) {
  const before = new Map(state.downloads.map((d) => [d.key, d.state]));
  state.downloads = list;
  for (const d of list) {
    if (state.pendingPlay === d.key && d.state === "downloading" && d.progress > 0) {
      state.pendingPlay = null;
      invoke("play_file", { path: d.path, growing: true }).catch(() => {});
    }
    const was = before.get(d.key);
    if (was === d.state) continue;
    if (d.state === "done" && was) {
      toast(t("downloads.finishedToast", { name: fileName(d.path) }));
      const c = state.clips.find((x) => x.key === d.key);
      if (c) c.file = d.path;
      if (state.pendingPlay === d.key) {
        state.pendingPlay = null;
        invoke("play_file", { path: d.path }).catch(() => {});
      }
    } else if (d.state === "failed" && was) {
      addProblem(d.error?.code || "ERR_DOWNLOAD_FAILED", d.error?.message, "download");
      toast(errorText(d.error?.code || "ERR_DOWNLOAD_FAILED").title, "error");
      if (state.pendingPlay === d.key) state.pendingPlay = null;
    }
  }
  const active = list.filter((d) => d.state === "queued" || d.state === "downloading").length;
  $("dlCount").hidden = active === 0;
  $("dlCount").textContent = active;
  if (state.view === "camera") renderClips();
  if (state.view === "downloads") renderDownloads();
  loadThumbs();
}

async function renderDownloads() {
  const folder = await invoke("output_dir").catch(() => "");
  $("dlFolder").textContent = folder;
  const rows = $("dlRows");
  const list = state.downloads.slice().reverse();
  $("dlTable").hidden = list.length === 0;
  $("dlEmpty").hidden = list.length > 0;
  $("btnClearFinished").disabled = !list.some((d) => !["queued", "downloading"].includes(d.state));
  rows.replaceChildren(
    ...list.map((d) => {
      const tr = el("tr");
      const name = el("td");
      const box = el("div", "dl-name");
      box.append(
        el("span", "dl-when", `${fmtDay(new Date(d.start * 1000), { day: "numeric", month: "short" })} ${fmtClock(d.start)}`),
        el("span", "dl-file", fileName(d.path)),
      );
      name.append(box);

      const status = el("td");
      const sbox = el("div", `dl-status${d.state === "failed" ? " dl-status--failed" : d.state === "done" ? " dl-status--done" : ""}`);
      const pct = Math.round((d.progress || 0) * 100);
      const label =
        d.state === "downloading" ? t("downloads.downloading", { percent: pct })
        : d.state === "failed" ? `${t("downloads.failed")}: ${errorText(d.error?.code || "ERR_DOWNLOAD_FAILED").title}`
        : t(`downloads.${d.state}`);
      sbox.append(el("span", "num", label));
      if (d.state === "downloading" || d.state === "queued") {
        const bar = el("div", "bar");
        const fill = el("span");
        fill.style.setProperty("--p", String(d.progress || 0));
        bar.append(fill);
        sbox.append(bar);
      }
      status.append(sbox);

      const act = el("td");
      const add = (label, cls, fn) => {
        const b = el("button", `btn ${cls}`, label);
        b.type = "button";
        b.addEventListener("click", fn);
        act.append(b);
      };
      if (d.state === "queued" || d.state === "downloading") add(t("downloads.cancel"), "btn--outline", () => invoke("cancel_download", { key: d.key }));
      if (d.state === "done") {
        add(t("downloads.play"), "btn--outline", () => {
          setView("camera");
          invoke("play_file", { path: d.path }).catch((e) => toast(errorText(asError(e).code).title, "error"));
        });
        add(t("downloads.showFile"), "btn--quiet", () => invoke("reveal_path", { path: d.path }));
      }
      if (d.state === "failed" || d.state === "cancelled") add(t("downloads.retry"), "btn--outline", () => queueDownload(d));
      tr.append(name, status, act);
      return tr;
    }),
  );
}

$("btnClearFinished").addEventListener("click", () => invoke("clear_finished_downloads"));
for (const id of ["btnOpenFolder", "btnOpenFolder2"]) {
  $(id).addEventListener("click", async () => invoke("reveal_path", { path: await invoke("output_dir") }));
}
for (const id of ["btnChangeFolder", "btnChangeFolder2"]) {
  $(id).addEventListener("click", async () => {
    state.dialogOpen = true;
    updateVideoVisibility();
    try {
      const picked = await dialog.open({ directory: true, defaultPath: await invoke("output_dir") });
      if (picked) {
        await invoke("set_output_dir", { path: picked });
        if (state.view === "settings") renderSettings();
        if (state.view === "downloads") renderDownloads();
        if (state.connected) loadClips();
      }
    } catch (e) {
      const err = asError(e);
      addProblem(err.code, err.message, "downloads folder");
      toast(errorText(err.code).title, "error");
    } finally {
      state.dialogOpen = false;
      updateVideoVisibility();
    }
  });
}

// ---------------------------------------------------------------- problems page
$("btnOpenLogs").addEventListener("click", async () => {
  const info = await invoke("app_info");
  invoke("reveal_path", { path: info.logsDir });
});
$("btnClearProblems").addEventListener("click", () => {
  state.problems = [];
  localStorage.setItem("problems", "[]");
  renderProblems();
});

// ---------------------------------------------------------------- settings
async function renderSettings() {
  const cfg = await invoke("settings_get").catch(() => ({}));
  const sel = $("language");
  sel.replaceChildren(...languageList().map((l) => {
    const o = el("option", null, l.name);
    o.value = l.code;
    return o;
  }));
  sel.value = languageCode();
  $("sFolder").textContent = await invoke("output_dir").catch(() => "");
  $("checkUpdates").checked = cfg.check_updates !== false;
}

async function renderAbout() {
  const info = await invoke("app_info").catch(() => ({}));
  const na = t("common.notAvailable");
  $("vApp").textContent = info.version || na;
  $("vPytapo").textContent = info.pytapo || na;
  $("vMpv").textContent = info.libmpv || na;
}

$("language").addEventListener("change", async (e) => {
  const code = await setLanguage(e.target.value);
  await invoke("settings_set", { patch: { language: code } }).catch(() => {});
  renderCalendar();
  if (state.connected) renderClips();
  renderSettings();
  renderQuality();
  showShell(state.connected);
  if (state.selected) $("clipsTitle").textContent = fmtDay(fromIso(state.selected));
});
$("btnLocales").addEventListener("click", () => invoke("open_locales_folder"));
$("checkUpdates").addEventListener("change", (e) => invoke("settings_set", { patch: { check_updates: e.target.checked } }));
$("btnDataFolder").addEventListener("click", async () => {
  const info = await invoke("app_info");
  invoke("reveal_path", { path: info.dataDir });
});
document.querySelectorAll("[data-link]").forEach((b) => b.addEventListener("click", () => invoke("open_link", { url: b.dataset.link })));

$("btnCheckNow").addEventListener("click", async () => {
  const btn = $("btnCheckNow");
  const out = $("updateStatus");
  btn.classList.add("is-loading");
  btn.disabled = true;
  out.textContent = t("settings.checking");
  try {
    const rel = await invoke("check_update", { manual: true });
    if (rel && rel.newer) {
      out.textContent = t("settings.updateFound", { version: rel.version });
      openUpdate(rel);
    } else out.textContent = t("settings.upToDate");
  } catch {
    out.textContent = t("settings.checkFailed");
  } finally {
    btn.classList.remove("is-loading");
    btn.disabled = false;
  }
});

// ---------------------------------------------------------------- update dialog
let pendingRelease = null;
function openUpdate(rel) {
  pendingRelease = rel;
  $("updTitle").textContent = t("update.title", { version: rel.version });
  $("updName").textContent = rel.name && rel.name !== `v${rel.version}` ? rel.name : "";
  $("updName").hidden = !$("updName").textContent;
  invoke("app_info").then((i) => ($("updBody").textContent = t("update.body", { current: i.version })));
  state.dialogOpen = true;
  updateVideoVisibility();
  $("updateDialog").showModal();
}
$("updateDialog").addEventListener("close", () => {
  state.dialogOpen = false;
  updateVideoVisibility();
});
$("btnUpdDownload").addEventListener("click", () => {
  if (pendingRelease) invoke("open_link", { url: pendingRelease.url });
  $("updateDialog").close();
});
$("btnUpdLater").addEventListener("click", () => $("updateDialog").close());
$("btnUpdSkip").addEventListener("click", () => {
  if (pendingRelease) invoke("settings_set", { patch: { skipped_version: pendingRelease.version } });
  $("updateDialog").close();
});

// ---------------------------------------------------------------- boot
async function boot() {
  const cfg = await invoke("settings_get").catch(() => ({}));
  await initI18n(cfg.language || "en");
  apply(document);
  state.quality = cfg.live_stream_quality === "SD" ? "SD" : "HD";
  renderQuality();
  loadProblems();
  renderProblemCount();
  await fillSavedLogin();
  showShell(false);
  const start = location.hash.slice(1);
  setView(["settings", "about", "problems"].includes(start) ? start : "login");
  await appWindow.show().catch(() => {});
  (state.dialogOpen ? null : $(($("host").value && $("username").value && $("password").value) ? "btnConnect" : "host"))?.focus();
  listen("downloads", (e) => onDownloads(e.payload));
  invoke("downloads").then(onDownloads).catch(() => {});
  pollPlayer();
  setTimeout(async () => {
    const rel = await invoke("check_update", { manual: false }).catch(() => null);
    if (rel && rel.newer) openUpdate(rel);
  }, 2500);
}

boot().catch((e) => {
  invoke("log_line", { line: `boot failed: ${e}` }).catch(() => {});
  appWindow.show().catch(() => {});
});
