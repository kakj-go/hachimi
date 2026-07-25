@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0reset-portable-data.ps1"
if errorlevel 1 (
  echo Failed to reset Hachimi portable data.
  pause
  exit /b 1
)
echo Hachimi portable data reset complete.
pause

