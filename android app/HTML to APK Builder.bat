@echo off
setlocal
cd /d "%~dp0"
python "%~dp0html_to_apk_builder.py"
if errorlevel 1 (
  echo.
  echo Failed to start with "python". Trying the Python launcher...
  py "%~dp0html_to_apk_builder.py"
  if errorlevel 1 (
    echo.
    echo The builder exited with an error. Press any key to close.
    pause >nul
  )
)
