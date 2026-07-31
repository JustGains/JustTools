@echo off
rem justvideo.cmd - run the bash justvideo script from cmd / PowerShell.
rem Calls Git Bash explicitly so it never falls through to WSL bash.exe.
setlocal
set "GITBASH=%ProgramFiles%\Git\bin\bash.exe"
if not exist "%GITBASH%" set "GITBASH=%ProgramFiles(x86)%\Git\bin\bash.exe"
if not exist "%GITBASH%" set "GITBASH=%LocalAppData%\Programs\Git\bin\bash.exe"
if not exist "%GITBASH%" echo justvideo: Git Bash not found; install Git for Windows. 1>&2 & exit /b 1
set "script=%~dp0justvideo"
set "script=%script:\=/%"
"%GITBASH%" "%script%" %*
