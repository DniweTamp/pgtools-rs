@echo off
setlocal EnableExtensions

echo ============================================================================
echo  PGTools Studio (pgtools-rs) -- Self-Contained Release Publisher
echo ============================================================================
echo.

REM Resolve repository root to the directory containing this batch script
set "ROOT_DIR=%~dp0"
if "%ROOT_DIR:~-1%"=="\" set "ROOT_DIR=%ROOT_DIR:~0,-1%"
cd /d "%ROOT_DIR%"

set "GUI_DIR=%ROOT_DIR%\crates\pgtools-gui"
set "RELEASE_DIR=%ROOT_DIR%\release"
set "ZIP_PATH=%ROOT_DIR%\PGTools-Studio-Release.zip"
set "SOURCE_ZIP_PATH=%ROOT_DIR%\PGTools-Studio-Source.zip"

REM ----------------------------------------------------------------------------
REM 1. Build React + TypeScript Frontend (crates\pgtools-gui\dist)
REM ----------------------------------------------------------------------------
echo [1/4] Building Tauri frontend assets...
cd /d "%GUI_DIR%"

if exist "%GUI_DIR%\node_modules" goto :skip_npm_install
echo       Installing npm dependencies...
call npm install
if errorlevel 1 goto :err_npm_install

:skip_npm_install
call npm run build
if errorlevel 1 goto :err_npm_build

if not exist "%GUI_DIR%\dist\index.html" goto :err_no_dist

REM ----------------------------------------------------------------------------
REM 2. Build Self-Contained GUI Executable (pgtools-gui.exe)
REM    Passing --features tauri/custom-protocol embeds crates\pgtools-gui\dist
REM    directly into the compiled .exe binary (no external dist/ folder needed).
REM ----------------------------------------------------------------------------
echo.
echo [2/4] Compiling self-contained GUI executable (pgtools-gui.exe)...
cd /d "%ROOT_DIR%"

cargo build --release -p pgtools-gui --features tauri/custom-protocol
if errorlevel 1 goto :err_cargo_gui

REM ----------------------------------------------------------------------------
REM 3. Build Standalone CLI Executable (pgtools.exe)
REM ----------------------------------------------------------------------------
echo.
echo [3/4] Compiling standalone CLI executable (pgtools.exe)...
cargo build --release -p pgtools-cli
if errorlevel 1 goto :err_cargo_cli

REM ----------------------------------------------------------------------------
REM 4. Assemble Self-Contained Release Folder & Optional Zip Archive
REM ----------------------------------------------------------------------------
echo.
echo [4/4] Staging release files into "%RELEASE_DIR%"...

if exist "%RELEASE_DIR%" rmdir /s /q "%RELEASE_DIR%"
mkdir "%RELEASE_DIR%"

copy /y "%ROOT_DIR%\target\release\pgtools-gui.exe" "%RELEASE_DIR%\PGTools-Studio.exe" >nul
if errorlevel 1 goto :err_copy_gui

copy /y "%ROOT_DIR%\target\release\pgtools.exe" "%RELEASE_DIR%\pgtools.exe" >nul
if errorlevel 1 goto :err_copy_cli

copy /y "%ROOT_DIR%\README.md" "%RELEASE_DIR%\README.md" >nul

REM Copy optional Oodle Kraken / NVTT DLLs if present in the repo root
for %%F in ("%ROOT_DIR%\oo2core*.dll" "%ROOT_DIR%\nvtt*.dll") do (
    if exist "%%~fF" copy /y "%%~fF" "%RELEASE_DIR%\" >nul
)

REM Create a ready-to-distribute ZIP archive next to the release folder
if exist "%ZIP_PATH%" del /f /q "%ZIP_PATH%"
powershell -NoProfile -ExecutionPolicy Bypass -Command "Compress-Archive -Path %RELEASE_DIR%\* -DestinationPath %ZIP_PATH% -Force" >nul 2>&1

REM Create a source archive from the current commit (tracked files only)
echo       Creating source archive from HEAD...
if exist "%SOURCE_ZIP_PATH%" del /f /q "%SOURCE_ZIP_PATH%"
git archive --format=zip --prefix=pgtools-rs/ --output="%SOURCE_ZIP_PATH%" HEAD
if errorlevel 1 goto :err_git_archive

echo.
echo ============================================================================
echo  Release build complete!
echo ============================================================================
echo   Release Folder : %RELEASE_DIR%
echo     - PGTools-Studio.exe  [Self-contained Tauri v2 GUI executable]
echo     - pgtools.exe         [Standalone CLI executable]
echo     - README.md           [Documentation and usage guide]
if exist "%ZIP_PATH%" echo   Release Zip    : %ZIP_PATH%
if exist "%SOURCE_ZIP_PATH%" echo   Source Zip     : %SOURCE_ZIP_PATH%
echo ============================================================================
endlocal
exit /b 0

:err_npm_install
echo [ERROR] npm install failed. Ensure Node.js and npm are installed.
exit /b 1

:err_npm_build
echo [ERROR] Frontend build failed.
exit /b 1

:err_no_dist
echo [ERROR] Expected frontend bundle at "%GUI_DIR%\dist\index.html" was not created.
exit /b 1

:err_cargo_gui
echo [ERROR] cargo build --release -p pgtools-gui failed.
exit /b 1

:err_cargo_cli
echo [ERROR] cargo build --release -p pgtools-cli failed.
exit /b 1

:err_copy_gui
echo [ERROR] Failed to copy pgtools-gui.exe to release folder.
exit /b 1

:err_copy_cli
echo [ERROR] Failed to copy pgtools.exe to release folder.
exit /b 1

:err_git_archive
echo [ERROR] Failed to create source ZIP with git archive. Ensure Git is installed and the current commit is available.
exit /b 1
