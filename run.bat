@echo off
rem Closes running Clockwork instances and MCP servers, rebuilds, and starts the newest Editor.
cd /d "%~dp0"

echo Closing running instances...
taskkill /F /IM clockwork.exe >nul 2>&1
taskkill /F /IM clockwork-mcp.exe >nul 2>&1
rem Give Windows a moment to release the executables so the linker can overwrite them.
timeout /t 1 /nobreak >nul

echo Building...
cargo build -p clockwork_editor -p clockwork_agent
if errorlevel 1 (
    echo.
    echo Build failed. Not starting.
    pause
    exit /b 1
)

echo Starting Clockwork...
start "" "%~dp0target\debug\clockwork.exe"
