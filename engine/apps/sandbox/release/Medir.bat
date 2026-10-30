@echo off
rem Forja: juega normalmente y guarda una medicion de rendimiento en forja_trace.txt
cd /d "%~dp0"
title Forja (medicion)
forja_fetch.exe fetch --play --missing --root "%~dp0."
set FORJA_NO_FETCH=1
set FORJA_TRACE_FILE=%~dp0forja_trace.txt
Forja.exe --frame-trace 60 %*
echo.
echo Medicion guardada en forja_trace.txt. Pega su contenido en el chat.
start "" notepad "%~dp0forja_trace.txt"
pause
