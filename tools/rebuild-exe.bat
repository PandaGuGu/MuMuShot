@echo off
chcp 65001 >nul
set PYTHONIOENCODING=utf-8
set PY=C:\Users\Administrator\.workbuddy\binaries\python\envs\mumu\Scripts\python.exe
"%PY%" "%~dp0rebuild_exe.py"
pause
