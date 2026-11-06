# PGTools Studio (`pgtools-rs`) — Rust + Tauri v2 PlatinumGames Asset Suite

A high-performance, self-contained Rust & Tauri v2 suite for inspecting, editing, converting, and repacking PlatinumGames archives, textures, 3D models, animations, cloth physics, binary XML, and particle effects across *Bayonetta 1/2/3*, *NieR:Automata*, *Astral Chain*, *Metal Gear Rising: Revengeance*, and *Vanquish*.

## System Requirements (Windows)
* **Windows 10 (version 1803+) or Windows 11 (64-bit)**.
* **Microsoft Edge WebView2 Runtime**: Pre-installed on Windows 11 and current Windows 10 systems. On older or stripped Windows 10 installations, install the [Microsoft Edge WebView2 Evergreen Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) so the Tauri desktop window can initialize.

## Quick Release Build (`publish_release.bat`)

Run `publish_release.bat` from the repository root on Windows to build and stage a release folder (`release\`), a binary archive (`PGTools-Studio-Release.zip`), and a source archive (`PGTools-Studio-Source.zip`):

```bat
.\publish_release.bat
```

To optionally bundle a local `oo2core_*_win64.dll` or `nvtt*.dll` from the repository root (when permitted for private/internal distribution):

```bat
.\publish_release.bat --with-oodle
```

This script:
1. Installs locked frontend dependencies (`npm ci` when `package-lock.json` is present) and builds the React + TypeScript frontend (`crates\pgtools-gui\dist`).
2. Compiles the **self-contained GUI executable** (`cargo build --release --locked -p pgtools-gui --features tauri/custom-protocol`), embedding the entire frontend bundle inside `PGTools-Studio.exe` so no external `dist/` folder is required.
3. Compiles the **standalone CLI executable** (`cargo build --release --locked -p pgtools-cli` -> `pgtools.exe`).
4. Packages `PGTools-Studio.exe`, `pgtools.exe`, and `README.md` (plus optional DLLs if `--with-oodle` was passed) into `release\` and `PGTools-Studio-Release.zip`.
5. Creates `PGTools-Studio-Source.zip` from the current `HEAD` commit using `git archive` (tracked files only).

## Included Executables

### 1. `PGTools-Studio.exe` (Desktop GUI)
* **Unified 3-Pane Studio Layout**:
  * **Asset Browser (Top-Left)** & **File Browser (Bottom-Left)**: Sort by `Name`, `Type`, `Modified`, or `Size`, filter by glob pattern or `Supported Only` (on by default), and right-click or drag-and-drop to add, remove, rename, reorder, replace, or extract files.
  * **Automatic `DAT` + `DTT` & `WTA` + `WTP` Pairing**: Opening either `.dat` or `.dtt` automatically loads its sibling archive, pairs `.wta` headers with `.wtp` payloads, and saves both containers atomically on `Ctrl+S`.
  * **Deep Nested Container Editing (`PKZ ↔ DAT/DTT ↔ EFF/IDD ↔ SCR ↔ Leaf`)**: Inspect and modify sub-files at any nesting depth (such as `.est` particle records inside `.eff` inside `.dat`, or `.wtb`/`.wmb` inside `.dat` inside `.pkz`) and commit changes directly back up the container chain.
  * **Game-Aware Texture Pipeline (`WTB`, `WTA`/`WTP`, `XT1`, `BNTX`, Wii U `GTX`, PC `DDS`)**:
    * Full `XT1` decoding & encoding parity with `XT1 Studio` (Tegra X1 GOB swizzling, `0x403` wide-pitch & single-mip ASTC pitch recovery, all 14 ASTC LDR footprints `4x4`..`12x12`, `Solidify` multi-source BFS alpha edge bleeding, and box mipmap chain generation).
    * Native Nintendo Switch `.bntx` (`BNTX` / `NX  ` / `BRTI`) and Wii U `.gtx` (`GX2` AddrLib MicroTiled/MacroTiled) decoding and export.
  * **Structured Editors (No Raw JSON Dumps)**: Dedicated editable tables and inspectors for `WMB1`/`WMB2`/`WMB3` (materials, shaders, samplers, batches, skeleton bones, header flags & bounding box), `EST` particle emitters, `CSV` tables, `BXM` binary XML, `CLP`/`CLH`/`CLW` cloth physics, `MOT` animations, `EXP` constraints, `COS` costume palettes, `ATT`, `OBI`, `MSQ`, and `COL`.
  * **Persistent Settings**: Saves theme (`Dark`/`Light`), recent files, last directory, splitter sizes, file filters, and external `oo2core_*.dll` path to `%APPDATA%\pgtools-studio\pgtools_settings.json`.

### 2. `pgtools.exe` (Command-Line Interface)
Run `pgtools.exe --help` for full CLI usage:

```bat
pgtools.exe dat-extract <files...> [-o <output_dir>]
pgtools.exe dat-create <input_dir> [-o <output.dat>]
pgtools.exe dat-ls <file.dat> [--yaml]
pgtools.exe pkz-extract <files...> [-o <output_dir>] [--oodle-dll <oo2core_8_win64.dll>]
pgtools.exe pkz-create <input_dir> -o <output.pkz> [--zstd]
pgtools.exe eff-extract <file.eff> [-o <output_dir>]
pgtools.exe eff-create <input_dir> [-o <output.eff>]
pgtools.exe scr-extract <file.scr> [-o <output_dir>]
pgtools.exe scr-create <input_dir> [-o <output.scr>]
pgtools.exe wtx-extract <file.wtb|file.wta> [file.wtp] [-o <output_dir>] [--png]
pgtools.exe wtx-create <input_dir> [-o <output.wtb>]
pgtools.exe wtb-convert-wiiu-pc <file.wtb|file.wta> [-o <output_pc.wtb>]
pgtools.exe wmb-tool <input.wmb|input.wmb3> [--cleanup-vertexes] [--cleanup-materials] [--swap-endian] [--export-obj <out.obj>]
pgtools.exe bxm-decode <input.bxm> [-o <output.xml>]
pgtools.exe bxm-encode <input.xml> [-o <output.bxm>]
pgtools.exe clp-convert <input.clp> [--swap-endian]
pgtools.exe clh-convert <input.clh> [--swap-endian]
pgtools.exe clw-convert <input.clw> [--swap-endian]
pgtools.exe mot-tool <input.mot> [--decode] [--swap-endian]
pgtools.exe exp-tool <input.exp> [--swap-endian]
```

## Oodle Kraken Support (`PKZ` Archives)
* `ZStandard` (`Zstd`) and uncompressed (`None`) `.pkz` archives work natively in pure Rust, and a built-in pure-Rust Oodle Kraken (`ooz`) fallback decompressor is included.
* For hardware-speed *Bayonetta 3* / *Astral Chain* `OodleKraken` compression and decompression, place `oo2core_*_win64.dll` (e.g., `oo2core_8_win64.dll` or `oo2core_9_win64.dll`) next to `PGTools-Studio.exe` / `pgtools.exe`, or select it via **Tools → Configure Oodle Kraken DLL (oo2core)...** in the GUI. (For security, only the application directory and an explicitly configured DLL path are searched.)
