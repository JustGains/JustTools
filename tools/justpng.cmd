@echo off
rem justpng.cmd - run the Bash justpng script from cmd / PowerShell.
setlocal
set "GITBASH=%ProgramFiles%\Git\bin\bash.exe"
if not exist "%GITBASH%" set "GITBASH=%ProgramFiles(x86)%\Git\bin\bash.exe"
if not exist "%GITBASH%" set "GITBASH=%LocalAppData%\Programs\Git\bin\bash.exe"
if not exist "%GITBASH%" echo justpng: Git Bash not found; install Git for Windows. 1>&2 & exit /b 1
set "script=%~dp0justpng"
set "script=%script:\=/%"
"%GITBASH%" "%script%" %*
