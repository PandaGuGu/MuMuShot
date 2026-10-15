@echo off
chcp 65001 >nul
set PYTHONIOENCODING=utf-8
set PY=C:\Users\Administrator\.workbuddy\binaries\python\envs\mumu\Scripts\python.exe
if "%~1"=="" (
  start "" "%PY%" "%~dp0mumu_shot.py"
) else (
  "%PY%" "%~dp0mumu_shot.py" %*
  echo.
  pause
)
