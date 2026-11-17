<div align="center">

<img src="assets/tapfeed-512.png" alt="TapFeed logo: a white Tapo-style pan and tilt camera on a blue circle" width="120" height="120" />

# TapFeed

**Tapo camera viewer for Windows: live view, SD card recordings, downloads, and pan and tilt.**

Previously named **Tapo-Viewer** ([old address](https://github.com/zaifears/tapo-viewer) redirects here).

[![Latest Release](https://img.shields.io/github/v/release/zaifears/TapFeed?color=00A4E4&label=Latest%20Release&style=for-the-badge)](https://github.com/zaifears/TapFeed/releases/latest)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%7C%2011-0078D6?style=for-the-badge&logo=windows&logoColor=white)](https://github.com/zaifears/TapFeed)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-B7410E?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![UI](https://img.shields.io/badge/UI-Tauri%202-24C8DB?style=for-the-badge&logo=tauri&logoColor=white)](https://tauri.app/)
[![Powered by PyTapo](https://img.shields.io/badge/Powered%20By-PyTapo%20v3.4.31-00A4E4?style=for-the-badge&logo=github&logoColor=white)](https://github.com/JurajNyiri/pytapo)
[![License](https://img.shields.io/badge/License-MIT-green?style=for-the-badge)](LICENSE)
[![Privacy](https://img.shields.io/badge/Camera%20traffic-Local%20only-success?style=for-the-badge)](#privacy)

[Download](https://github.com/zaifears/TapFeed/releases/latest) | [Screens](#screens) | [Features](#features) | [Cameras](#supported-tapo-cameras) | [FAQ](#faq) | [Languages](#languages)

</div>

## What is TapFeed?

TapFeed (previously named Tapo-Viewer) is a free, open-source Windows app for TP-Link Tapo security cameras. It shows the live video, lists the recordings on the camera's microSD card by day, downloads them to your PC, and turns pan and tilt cameras. It talks to the camera directly on your home network, with no cloud subscription.

## Download

1. Get **`TapFeed.exe`** from [Releases](https://github.com/zaifears/TapFeed/releases/latest). Versions up to 1.1.3 are named `Tapo-Viewer.exe`.
2. Double-click it. Nothing to install.

## Screens

**Sign in**

<img src="https://i.ibb.co.com/bMzFcWrq/login-tapfeed.png" alt="TapFeed sign-in screen: camera IP address with a Test button, Camera Account username and password, optional Tapo app password" width="100%" />

**Camera**

<img src="https://i.ibb.co.com/rKn0GZ12/feed-tapfeed.png" alt="TapFeed camera screen: Tapo live view, a month calendar marking days with SD card recordings, and the list of that day's recordings" width="100%" />

**Downloads**

<img src="https://i.ibb.co.com/8nr97KYb/download-tapfeed.png" alt="TapFeed downloads screen: saved Tapo recordings with progress, play and show in folder" width="100%" />

## Features

| | |
|---|---|
| **Live view** | HD or SD, with sound, full screen and screenshots |
| **Pan and tilt** | Arrow buttons and saved positions, only on cameras with a motor |
| **SD card calendar** | Days with recordings are marked; pick one to see its clips |
| **Downloads** | Saved as `.ts` video, one at a time, with progress and cancel |
| **Plain-language problems** | Every error says what happened and what to try |
| **Remembers you** | Passwords stay in Windows Credential Manager, only if you ask |
| **Update notice** | A small prompt when a new version is out; you can turn it off |
| **Your language** | All text comes from one JSON file per language |

**Lighter than Tapo-Viewer 1.x**, measured on the same PC: a 62 MB download instead of 264 MB, opens in about 3 seconds instead of 24 to 47, and no longer unpacks 610 MB into the temp folder on every start.

## Supported Tapo cameras

Tapo cameras that offer a Camera Account and RTSP live video, for example:

- **Indoor:** C100, C110, C200, C210, C220, C225, TC70
- **Outdoor:** C310, C320WS, C325WB, C500, C510W, C520WS

Pan and tilt controls appear only on models with a motor, such as the C200, C210, C225 and C500 (tested on the C200). Battery cameras usually have no RTSP; see [TP-Link's list](https://www.tp-link.com/us/support/faq/4465/).

## Before you connect

Tapo cameras use two logins. TapFeed may need both.

| | Camera Account | Tapo app password |
|---|---|---|
| **What** | A username and password made inside the camera's settings | The password you use in the Tapo app |
| **Where** | Tapo app, camera, Settings, Advanced settings, Camera Account | Your TP-Link ID |
| **Used for** | Live video | Listing and downloading recordings on some cameras |

The camera's IP is in the Tapo app under Device info. Use **Test** beside the IP to check it. If your Tapo app password differs from the Camera Account password, tick **My Tapo app password is different**.

## FAQ

**Is TapFeed free?**
Yes. It is open source under the MIT License.

**Do I need Tapo Care or the internet?**
No. TapFeed reads the camera's own microSD card over your home network.

**Can I download Tapo SD card recordings to my PC?**
Yes. Pick a day in the calendar and select Download. Files are `.ts` video, which VLC, mpv and most players open.

**Is my password safe?**
Passwords go only to your camera. If you tick Remember, they are kept in Windows Credential Manager, never in a plain file.

**What happened to Tapo-Viewer?**
It is now called TapFeed. On first start, TapFeed copies your Tapo-Viewer settings, saved passwords and language files, and keeps your downloads where they are.

## Languages

**New languages are very welcome.** Each language is one JSON file, so no coding is needed.

1. Copy [`desktop/ui/locales/en.json`](desktop/ui/locales/en.json) to your language code, for example `bn.json`.
2. Translate the text. Keep the keys and anything in `{braces}`.
3. Add it to [`desktop/ui/locales/index.json`](desktop/ui/locales/index.json) and open a pull request.

To try a translation without building, use **Settings, Open language folder** in the app, copy `_template.json` to your code, translate it and restart.

## Privacy

TapFeed talks to your camera over your own network. Passwords are never sent anywhere else. The only internet request is the optional update check to GitHub, which sends no camera details.

## Build from source

Needs Rust (MSVC), Visual Studio 2022 C++ Build Tools, Python 3.10+ with `pip install -r requirements.txt`, and a 64-bit `libmpv-2.dll` in `vendor/libmpv/`.

```powershell
git clone https://github.com/zaifears/TapFeed.git
cd TapFeed
build_desktop.bat
```

Output: `dist\TapFeed.exe`. To update pytapo, run `.venv\Scripts\python -m pip install -U pytapo`, then `build_desktop.bat` again.

## Thanks

Camera connection by **[pytapo](https://github.com/JurajNyiri/pytapo)** from [Juraj Nyíri](https://github.com/JurajNyiri). Video by [mpv](https://mpv.io).

Thank you for using TapFeed. Loved the project? [Buy me a coffee please!](https://shahoriar.bd/thanks)

Found a bug? Open the [issues page](https://github.com/zaifears/TapFeed/issues) and select **New issue**.

Made by [Shahoriar Hossain](https://github.com/zaifears). [MIT License](LICENSE). TapFeed is not affiliated with TP-Link; Tapo is a trademark of TP-Link.

<p align="center"><img src="assets/free_palestine.png" alt="Free Palestine" width="400" /></p>
