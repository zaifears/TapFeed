# Tapo-Viewer desktop (Tauri) — spec

Replaces the customtkinter UI. Design and evaluation: `docs/hybrid-rust-evaluation.md`.

## Layout of the repo

| Path | What |
|---|---|
| `desktop/` | Tauri 2 app (Rust). Builds `Tapo-Viewer.exe` |
| `desktop/ui/` | Plain HTML/CSS/JS, no bundler |
| `desktop/ui/locales/*.json` | All UI text. One file per language |
| `helper/tapo_helper.py` | Python helper: pytapo only, JSON lines over stdin/stdout |
| `helper/build_helper.bat` | Freezes the helper to `vendor/helper/tapo-helper.exe` |
| `vendor/libmpv/libmpv-2.dll`, `vendor/helper/tapo-helper.exe` | Packed into the exe at build time (git-ignored) |

## Behaviour

**Sign in.** Camera IP, Camera Account username + password, optional Cloud password (Tapo app
password), "Remember me" (Windows Credential Manager). Connect = helper `connect`, which tries
the two login methods (below) and reports which worked; it is remembered and tried first next
time. Then the live view starts.

**Login fallback (fixes the two-account report).** At most 2 attempts per click, stop on lockout:
- Cloud password given: `admin` + Cloud password, then Camera Account.
- Not given: Camera Account, then `admin` + the same password.

**Camera screen.** Player (live or clip) with play/pause, stop, back to live, HD/SD, volume, mute,
fullscreen. Calendar: month grid, days with recordings marked, today and selected day shown.
Clip list for the selected day: start–end time, length, motion/continuous, state
(on camera / downloading n% / downloaded), Play and Download. Play on a clip that is not
downloaded downloads it first, then plays it.

**Downloads.** One at a time (the camera allows one stream). Queue, progress, cancel, open
folder, play. Output is the camera's `.ts` as-is, named `tapo_YYYYMMDD_HHMMSS_HHMMSS.ts`;
existing `.ts`/`.mp4` with that name count as downloaded.

**Diagnostics.** Separate page. Each problem: when, a plain title, one-sentence explanation,
what to do. "Technical details" folds out; "Copy details" copies them (passwords never included).
"Open log folder". No live console.

**Settings and about.** Language, download folder, check for updates on startup, check now,
versions (app, pytapo, libmpv), credits.

**Update check.** Startup, background, 3 s timeout, GitHub `releases/latest` for
`zaifears/TapFeed`. Newer tag → dialog: Download (opens release page), Later, Skip this
version. Silent on any error.

**Languages.** `ui/locales/en.json` is the source. Adding a language = copy it to `xx.json`,
translate the values, add it to `locales/index.json`. Users can also drop a `xx.json` into
`%LOCALAPPDATA%\Tapo-Viewer\locales\` without a rebuild. Missing keys fall back to English.

## Data

- Settings: `%LOCALAPPDATA%\Tapo-Viewer\config.json` (same file the Python app used; unknown
  keys kept).
- Passwords: Credential Manager, service `Tapo-Viewer`, accounts `camera:<user>@<host>` and
  `cloud:<user>@<host>`; entries saved by the Python app are read once and re-saved.
- Runtime: `%LOCALAPPDATA%\Tapo-Viewer\runtime\<hash>\` (libmpv, helper).
- Logs: `%LOCALAPPDATA%\Tapo-Viewer\logs\` with passwords removed.

## Helper protocol (v1)

One JSON object per line. Request `{"id":n,"cmd":..., ...}` → `{"id":n,"ok":true,"result":...}`
or `{"id":n,"ok":false,"code":"ERR_…","message":"…"}`. Events: `{"event":"download_progress",...}`.

| cmd | args | result |
|---|---|---|
| `hello` | | `{protocol, pytapo}` |
| `connect` | host, username, password, cloud_password?, prefer? | `{model, alias, mac?, firmware?, login, time_correction}` |
| `recording_dates` | | `["YYYY-MM-DD", …]` |
| `list_recordings` | date | `[{start, end, motion}]` (unix seconds) |
| `download` | start, end, path | progress events; `{path}` |
| `cancel` | | `{}` |
| `disconnect` | | `{}` |

## Done when

- One exe, no install, runs on a clean Windows 10/11 PC with WebView2.
- Sign in (both login methods), live view HD/SD with audio, calendar marks days, clip plays,
  download with progress/cancel, diagnostics page, update dialog, language from JSON.
- No spike text, no console, no unexplained error.

## Build

```
build_desktop.bat
```

Freezes the helper (`helper/build_helper.py`), builds `desktop/` and copies the result to
`dist/Tapo-Viewer.exe`. Prerequisites are listed at the top of the script.

**Upgrading pytapo:** `.venv\Scripts\python -m pip install -U pytapo`, then `build_desktop.bat`. Nothing
in Rust or the UI changes unless pytapo renames something the helper calls.

## Adding a language

1. Copy `desktop/ui/locales/en.json` to `<code>.json` (for example `bn.json`) and translate the
   values. Keep the keys and the `{placeholders}`; set `_meta.name`.
2. Add `{ "code": "bn", "name": "বাংলা" }` to `desktop/ui/locales/index.json` and rebuild.

Without rebuilding: Settings, Open language folder, copy `_template.json` to `bn.json`, translate,
restart the app. The language appears in the list.
