@echo off
rem just.cmd - run the Node just selector from cmd / PowerShell.
setlocal
where node >nul 2>nul || (echo just: Node.js not found on PATH. 1>&2 & exit /b 1)
node "%~dp0just.js" %*
exit /b %errorlevel%
