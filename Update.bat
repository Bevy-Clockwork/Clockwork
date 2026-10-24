@echo off
rem Pulls the newest Clockwork from GitHub into this checkout. Run run.bat afterwards to rebuild.
cd /d "%~dp0"

where git >nul 2>&1
if errorlevel 1 (
    echo Git was not found. Install it from https://git-scm.com and try again.
    pause
    exit /b 1
)

for /f "delims=" %%b in ('git rev-parse --abbrev-ref HEAD') do set BRANCH=%%b
echo Updating branch %BRANCH%...

rem Only fast-forwards: local commits or changes are never merged over or thrown away.
git pull --ff-only
if errorlevel 1 (
    echo.
    echo Update failed. Commit or stash local changes, or switch to main, and try again.
    pause
    exit /b 1
)

echo.
echo Up to date. Run run.bat to rebuild and start Clockwork.
pause
