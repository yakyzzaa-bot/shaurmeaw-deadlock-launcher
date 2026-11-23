@echo off
chcp 65001 >nul
set RUSTUP_HOME=C:\rustup
set CARGO_TARGET_DIR=C:\DeadlockLauncher\target
set RUSTFLAGS=-C link-self-contained=yes
set PATH=C:\WinLibs\mingw64\bin;%PATH%
cd /d "%~dp0"
cargo +stable-x86_64-pc-windows-gnu build --release --offline
if errorlevel 1 (
    echo Ошибка сборки!
    pause
    exit /b 1
)
copy /Y "C:\DeadlockLauncher\target\release\ShaurMeaw.exe" "C:\DeadlockLauncher\ShaurMeaw.exe" >nul
echo Готово: C:\DeadlockLauncher\ShaurMeaw.exe
pause
