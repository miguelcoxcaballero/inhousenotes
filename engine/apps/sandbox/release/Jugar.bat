@echo off
rem Forja: descarga el contenido la primera vez y abre el juego.
cd /d "%~dp0"
title Forja
forja_fetch.exe fetch --play --missing --root "%~dp0."
if errorlevel 1 echo Algunos paquetes no se descargaron; el juego usara sustitutos. Vuelve a ejecutar Jugar.bat para reintentarlo.
set FORJA_NO_FETCH=1
Forja.exe %*
if errorlevel 1 (
  echo.
  echo Forja se cerro con un error. Revisa los requisitos en LEEME.txt.
  pause
)
