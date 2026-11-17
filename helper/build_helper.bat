@echo off
rem Rebuild the pytapo helper after "pip install -U pytapo".
cd /d "%~dp0.."
".venv\Scripts\python.exe" helper\build_helper.py || exit /b 1
