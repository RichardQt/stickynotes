@echo off
rem Build the release binary and keep a copy in the repository root, where
rem `cargo clean` cannot reach it. Run from the repository root.
setlocal
cargo build --release -p stickynotes-app
if errorlevel 1 exit /b %errorlevel%
copy /Y target\release\stickynotes.exe stickynotes.exe >NUL
if errorlevel 1 (
    echo Could not copy stickynotes.exe - it may be running. Close the app and try again.
    exit /b 1
)
echo Copied to %CD%\stickynotes.exe
