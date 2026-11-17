@echo off
rem Builds dist\TapFeed.exe: one file, no install.
rem Needs: Rust (MSVC), VS 2022 C++ Build Tools, .venv with requirements + pyinstaller,
rem        and vendor\libmpv\libmpv-2.dll (64-bit, shinchiro mpv-dev build).
cd /d "%~dp0"
if not exist "vendor\libmpv\libmpv-2.dll" (
  echo [ERROR] vendor\libmpv\libmpv-2.dll is missing.
  exit /b 1
)
echo [1/2] Freezing the pytapo helper...
".venv\Scripts\python.exe" helper\build_helper.py || exit /b 1
echo [2/2] Building the app...
pushd desktop
cargo build --release || (popd & exit /b 1)
popd
if not exist dist mkdir dist
copy /y "desktop\target\release\tapfeed.exe" "dist\TapFeed.exe" >nul
echo Done: dist\TapFeed.exe
