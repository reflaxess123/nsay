@echo off
REM Restart nsay in dev mode. Sets cwd to the repo root so Tauri CLI finds
REM src-tauri/tauri.conf.json, prepends cargo to PATH, and points the
REM model resolver at the repo's models/ dir (the dev exe lives in
REM target/debug/ which has no models/ next to it).
cd /d "%~dp0"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
set "NSAY_MODELS_DIR=%~dp0models"
".\ui\node_modules\.bin\tauri.cmd" dev
