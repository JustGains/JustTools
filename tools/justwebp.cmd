@echo off
rem justwebp.cmd - run the Bash justwebp script from cmd / PowerShell.
setlocal
set "GITBASH=%ProgramFiles%\Git\bin\bash.exe"
if not exist "%GITBASH%" set "GITBASH=%ProgramFiles(x86)%\Git\bin\bash.exe"
if not exist "%GITBASH%" set "GITBASH=%LocalAppData%\Programs\Git\bin\bash.exe"
if not exist "%GITBASH%" echo justwebp: Git Bash not found; install Git for Windows. 1>&2 & exit /b 1
set "script=%~dp0justwebp"
set "script=%script:\=/%"
"%GITBASH%" "%script%" %*
