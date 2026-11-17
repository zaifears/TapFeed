# Tapo-Viewer: Rust UI + Python pytapo helper (evaluation)

Status: evaluation only, nothing built. Written 2026-11-17 against v1.1.4 / pytapo 3.4.31.

**Decided (2026-11-17):** Tauri 2 · `.ts` output only, no MP4 conversion and no ffmpeg · Windows 10/11 only ·
the login fix ships with the new app, not in the Python version · add an "update available" check.

## Verdict

**Feasible.** The plan works, with one correction to the split of work:

> Recording **downloads cannot move to Rust.** The camera's SD-card stream (port 8800) is
> encrypted with pytapo's own protocol, and porting that is exactly the work the hybrid avoids.
> Python fetches the bytes; Rust owns everything around them: queue, progress, cancel,
> file naming, the library view, playback.

Everything else in the request maps cleanly:

| Job | Owner | How |
|---|---|---|
| Window, screens, calendar, clip list, settings, error dialogs | Rust | Tauri 2 |
| Live view (RTSP) | Rust | libmpv in an embedded native window |
| Playing downloaded clips | Rust | same libmpv player |
| RTSP credential check | Rust | `retina` crate (RTSP DESCRIBE + digest auth), replaces ffprobe.exe |
| Download queue, progress, cancel, file management | Rust | drives the helper; clips stay `.ts` |
| Update-available check | Rust | one GitHub request at startup (see below) |
| Saved passwords | Rust | `keyring` crate (same Windows Credential Manager) |
| Camera login, recording dates/list, fetching clip bytes | Python | pytapo, unchanged |

## Architecture

```
TapoViewer.exe (Rust, single file)
 ├─ UI (Tauri 2 webview)
 ├─ libmpv player  ── rtsp://camera:554/stream1   (live, direct; no Python involved)
 └─ tapo-helper.exe (frozen Python + pytapo), child process
        stdin  ← {"id":7,"cmd":"list_recordings","date":"2026-11-17"}
        stdout → {"id":7,"ok":true,"result":[...]}
        stdout → {"event":"download_progress","job":3,"progress":120,"total":300}
```

### Helper protocol (whole surface: ~6 commands)

One JSON object per line. Requests carry an `id`; responses echo it. Progress is an
unsolicited `event`. Python `print` / logging goes to **stderr only** so stdout stays clean.

| Command | Wraps | Returns |
|---|---|---|
| `hello` | `pytapo.version` | helper version, pytapo version, protocol version |
| `connect` | `Tapo(...)` with fallback (below), `getBasicInfo`, `getTimeCorrection` | device info, which login worked |
| `recording_dates` | `getRecordingsList` | `["2026-11-17", ...]` |
| `list_recordings` | `getRecordings(date)` | start/end/type per clip (the parsing in `camera_backend.get_recordings`) |
| `download` | `Downloader(output="ts")` | progress events, then the .ts path |
| `cancel` / `disconnect` / `shutdown` | | ack |

Errors return `{"ok":false,"code":"ERR_CLOUD_AUTH","message":"...","technical":"..."}`.
The existing `errors.py` classifier can run inside the helper so the same codes reach the UI.

Passwords travel only over stdin, never in command-line arguments or environment variables
(both are visible to other processes).

The helper is about 300–400 lines: `camera_backend.py`'s pytapo parts plus a read loop.

## The login bug from the user report: fixed by design

**Confirmed in the current code.** [camera_backend.py](../camera_backend.py) `connect()`:

```python
user_for_cloud = (cloud_email.strip() if cloud_email else self.username) or "admin"
self.tapo = Tapo(host=..., user=user_for_cloud, password=self.password,
                 cloudPassword=self.cloud_password, ...)
```

`app.py` never passes `cloud_email`, so pytapo's control login always uses the **Camera Account**
username and password. Inside pytapo the legacy login sends `user` + `password`; `cloudPassword`
is only used for the media stream and the newer TPAP login. On cameras where the control API only
accepts `admin` + Cloud password, login fails, exactly as reported. `connect_cloud()` has the same
code and nothing calls it.

**Fix (goes in the helper's `connect`):**

1. If the user entered a separate Cloud password: try `admin` + Cloud password first (pytapo's
   documented method), then Camera Account user/pass.
2. If not: try Camera Account user/pass (today's behaviour), then `admin` + the same password.
3. Stop immediately on a lockout/"too many attempts" error. Never more than 2 attempts per click,
   because failed logins count toward the camera's lockout.
4. Report which method worked; Rust saves it in config and tries that method first next time,
   so a normal reconnect costs one login.

This fix is about 25 lines. **Decided:** it ships with the new app, not as a Python update.

## Rust UI options

| | Tauri 2 | egui (eframe) | Slint |
|---|---|---|---|
| "Simple, professional look" | Best: HTML/CSS, any design | Functional; takes effort to look polished | Good, declarative styling |
| Embedded live video | libmpv in a child window placed over a reserved area | libmpv child window or render into a texture | Same as egui |
| Draw controls on top of video | No (native window sits above the webview) | Yes with texture render | Yes with texture render |
| Exe size (UI only) | ~8–12 MB (uses Windows' WebView2) | ~5–8 MB | ~5–8 MB |
| Needs on the PC | WebView2 runtime: built into Windows 11, present on nearly all updated Windows 10 | Nothing | Nothing |
| Learning curve | Rust backend + HTML/JS front end | Rust only | Rust + Slint markup |

**Decided: Tauri 2.** The UI is your main pain point, and HTML/CSS is the fastest route
to a clean professional design. The current layout already puts controls in bars around the video,
so the "nothing on top of the video" limit costs nothing.

**Choose egui instead if** you want zero dependence on WebView2 (for example very old,
never-updated Windows 10 PCs) or want overlay controls on the video.

**Why WebView2 `<video>` alone is not enough:** it cannot play RTSP, and H.265 depends on a paid
Windows extension. libmpv plays RTSP, H.264 and H.265 with hardware decoding and low latency, so it
handles live and recorded video in both UI options.

## Single portable exe (no install)

Approach: the Rust exe embeds its parts (`include_bytes!`) and on first launch unpacks them to
`%LOCALAPPDATA%\TapoViewer\runtime\<version-hash>\`, then reuses that folder. This is what
PyInstaller `--onefile` already does today, so the user experience is the same: download one exe,
double-click.

Sizes are rough estimates, not measured; "packed" is the size compressed inside the exe.

| Part | Unpacked | Packed | Notes |
|---|---|---|---|
| Rust app + UI | ~10 MB | ~5 MB | |
| libmpv-2.dll | ~90–110 MB | ~30–35 MB | ffmpeg libraries are built in; replaces VLC + plugins |
| tapo-helper.exe (Python + pytapo) | ~30–40 MB | ~25–35 MB | python-kasa pulls in aiohttp/cryptography; Nuitka or PyInstaller |
| **Total download** | | **~60–75 MB** | today: 251 MB |

No ffmpeg at all: pytapo's `output="ts"` writes the camera's MPEG-TS as-is without calling
ffmpeg (checked in pytapo 3.4.31 `media_stream/convert.py`). ffmpeg.exe, ffprobe.exe, ffplay.exe,
vlc.exe, libvlc and the VLC plugins all go away.

**`.ts` caveat:** the camera's audio is G.711. The app's own player (libmpv), VLC, mpv and
PotPlayer play these files fine; the built-in Windows "Media Player" / "Films & TV" may play video
with no sound or refuse the file. "Open in external player" should therefore prefer VLC/mpv/PotPlayer
when installed. An optional "Export as MP4" can be added later if users ask.

Risks:
- **Antivirus false positives.** Self-unpacking exes (PyInstaller onefile today) get flagged
  sometimes. Code-signing the exe is the real fix; an unpacked "portable folder" zip is the
  fallback.
- **First launch** takes 1–3 s longer while unpacking; later launches are fast.

## Upgrading pytapo

| Option | How | Single exe? |
|---|---|---|
| **A. Rebuild helper (recommended)** | `pip install -U pytapo`, run `build_helper.bat`, rebuild the exe. Rust code untouched. | Yes |
| B. In-app pytapo update | pytapo is a pure-Python wheel; the helper could load a newer copy from `%LOCALAPPDATA%` if present | Yes, but breaks if a new pytapo adds a dependency, and runs downloaded code: not recommended for v1 |

With A, a pytapo upgrade is about two minutes of build work. Rust only changes if pytapo changes
a function the helper calls.

## Update-available check

Possible and small (about 80 lines of Rust + a dialog).

**Behaviour**
1. At startup, after the window is shown, one background request to
   `https://api.github.com/repos/zaifears/tapo-viewer/releases/latest` (3 s timeout).
2. Compare its `tag_name` (e.g. `v1.2.0`) with the version built into the exe, using semantic
   versioning (so `v1.10.0` > `v1.9.0`). Drafts and pre-releases are ignored by this endpoint.
3. If newer, a small dialog: **"Tapo-Viewer v1.2.0 is available"**, the release title, and
   three buttons: **Download** (opens the GitHub release page in the browser), **Later** (shows
   again next launch), **Skip this version** (stays quiet until an even newer release).
4. Offline, GitHub down, rate-limited (60 requests/hour per IP, unauthenticated) or any error:
   say nothing. The check never blocks or delays the app.
5. A "Check for updates on startup" switch in Settings/About, on by default, and a manual
   "Check now" button in About.

**Deliberately not included:** automatic download and replace. Tauri's updater plugin
expects an installer and signed update bundles; for a portable exe, opening the release page is
simpler and safer.

**Privacy:** the request sends only the PC's IP address and a generic `User-Agent:
Tapo-Viewer/<version>` to GitHub. No camera data, no IDs. Mention this in the README.

**Release rule this depends on:** tags must stay in the `vMAJOR.MINOR.PATCH` form already used
(`v1.0.0` … `v1.1.3`), and the version in the Rust build must be bumped with each release.

## Benefits you would get

- **UI problems go away at the root:** no customtkinter quirks, no Tk HWND tricks for VLC,
  no PyInstaller + LibVLC init failures (`9c3affb`), no UI freezes from the Python GIL.
- **Better live view:** libmpv is lower latency than VLC and handles H.265 and reconnects well.
- **Smaller download:** roughly 60–75 MB instead of 251 MB.
- **Faster downloads:** no conversion step after each clip.
- **Faster start:** a native window opens immediately; the Python helper starts in the
  background while you type the password.
- **pytapo stays drop-in upgradable**, and a camera-side failure can never freeze or crash the
  UI. If the helper dies, Rust restarts it and shows a clear error.
- **The login fallback** fixes the reported two-account bug.

## Costs and risks

- **You maintain two languages and two build steps** (Rust + frozen Python helper).
- **libmpv embedding is the riskiest piece** in Tauri. Prove it first (spike below).
- **Saved passwords:** Python's `keyring` and the Rust `keyring` crate name Credential Manager
  entries differently. Rust reads the old entry once and re-saves it, so users don't re-type
  passwords. `config.json` can be read as-is.
- **Rewrite effort:** the UI is the bulk (`app.py` 1,874 lines, player 777, calendar 247,
  about 356). Rough estimate for one person new to Rust: 3–5 weeks to parity; less if familiar
  with Tauri.

## Suggested order (when you decide to build)

1. **Spike (2–3 days):** Tauri window + libmpv playing the live RTSP stream and a downloaded
   `.ts` clip, packed into one exe. If this fails, switch to egui before writing any real UI.
2. Helper: JSON loop + `connect`/`recording_dates`/`list_recordings`/`download`, tested from a
   terminal by typing JSON.
3. Rust: helper process manager (start, restart, request/response matching, progress events).
4. UI screens: login → dashboard (live view, calendar, clip list) → downloads → about/settings.
5. Update-available check.
6. Packaging: embedded parts, unpack-on-first-run, code signing.
7. Credential migration, then retire the Python UI.

## Spike prerequisites (this PC, checked 2026-11-17)

| Needed | Status |
|---|---|
| Node.js / npm | installed (v26.5.1 / 11.17.0) |
| WebView2 runtime | installed (157.0.4341.46) |
| Rust toolchain (rustup, `x86_64-pc-windows-msvc`) | installed (rustc/cargo 1.100.0) |
| Visual Studio 2022 Build Tools, "Desktop development with C++" + Windows SDK | installed (17.14.45, SDK 10.0.26100.0); test program builds and runs |
| Tauri CLI (`cargo install tauri-cli` or `npm i -D @tauri-apps/cli`) | missing, installs with the project |
| libmpv-2.dll (64-bit, e.g. shinchiro's mpv-dev builds) | missing, downloaded for the spike |

## Spike results (2026-11-17, branch `spike/tauri-libmpv`, folder `tauri-spike/`)

**Result: the riskiest part works.** A Tauri 2 window with libmpv drawing into a native child window
over the page's video area, all in one portable exe.

| Check | Result |
|---|---|
| Single exe, no install | `tapo-viewer-spike.exe` **40.7 MB** (UI + libmpv 0.42 packed with zstd). Runs from any folder. |
| libmpv unpack on first run | 1.8 s, to `%LOCALAPPDATA%\TapoViewer\runtime\<hash>\`; later runs reuse it |
| Video inside the Tauri window | Yes: drawn exactly in the reserved area, page controls around it stay visible and clickable |
| Recorded clip (`.ts`, 1080p H.264 + AAC from a real Tapo recording) | Plays, **hardware decoding (d3d11va)**, 15 fps, 0 dropped frames |
| Warm start | window + player ready in about 1.5 s; clip starts in under 1 s |
| Cold first start | about 6 s until the player is ready (first WebView2 profile + unpack) |
| Build time | first release build about 12 min (all Tauri crates, max zstd, fat LTO); later builds much faster |

**Not yet verified (needs you / a real camera):**
- Live RTSP from the camera (enter IP + Camera Account in the left panel, "Start live view").
- A `.ts` straight from pytapo with the camera's G.711 audio (the sample had AAC because ffmpeg
  cannot write G.711 into `.ts`).
- Window resize, maximise, moving between monitors with different scaling.

Projected final size: 40.7 MB + Python helper (25–35 MB) ≈ **65–75 MB**, versus 251 MB today.
