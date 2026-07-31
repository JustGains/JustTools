@echo off
rem justzip - archive a Git working tree into the current directory while honoring ignores.
pwsh.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0justzip.ps1" %*
exit /b %ERRORLEVEL%
