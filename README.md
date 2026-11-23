# PGTools Studio (`pgtools-rs`)

Desktop asset suite (`PGTools-Studio.exe`) and command-line toolkit (`pgtools.exe`) for inspecting, editing, converting, and repacking PlatinumGames game assets. Written in Rust and Tauri v2 as a native rewrite of [`bayonetta_tools`](https://github.com/Kerilk/bayonetta_tools).

Supports formats from *Bayonetta*, *Bayonetta 2*, *Bayonetta 3*, *NieR:Automata*, *Astral Chain*, *Metal Gear Rising: Revengeance*, *Vanquish*, *The Wonderful 101*, and *Anarchy Reigns*.

## Supported Formats

| Category | Formats | Capabilities |
| :--- | :--- | :--- |
| Archives | `DAT`, `DTT`, `EVN`, `PKZ`, `EFF`, `IDD`, `SCR` | Extract, repack, re-align, nested container editing, automatic `DAT`+`DTT` pairing |
| Models | `WMB`, `WMB2`, `WMB3`, `MDB` | 3D viewport preview, material and shader editing, bone/mesh inspection, Wii U/PC endian swap, OBJ export, vertex/material cleanup |
| Textures | `WTB`, `WTA`/`WTP`, `DDS`, `GTX`, `BNTX`, `XT1` | Decode, encode, mipmap generation, cubemap skybox preview, Wii U `GTX` to PC `DDS` conversion, `CRC32 & 7F` hash remapping |
| UI & Atlases | `UVD` | Group and sprite inspection, per-group PNG extraction, automatic atlas repacking |
| XML & Data | `BXM`, `GAD`, `GAArea`, `CSV`, `EST`, `COL`, `MSQ`, `COS`, `OBI`, `ATT`, `FLS`, `FLA`, `CTA`, `LIT` | Binary XML tree editor and XML round-trip, structured record tables |
| Animation & Physics | `MOT`, `MOT2`/`B3MOT`, `EXP`, `EXP2`, `CLP`, `CLH`, `CLW` | Frame decoding, bone remapping, endian conversion, cloth/constraint editing |

## PGTools Studio (GUI)

`PGTools-Studio.exe` bundles the React/TypeScript frontend inside a single Windows executable (requires the Microsoft Edge WebView2 Runtime, pre-installed on Windows 11 and updated Windows 10).

- **3-Pane Browser and Deep Nesting**: Browse local directories alongside nested archive trees (`PKZ` > `DAT`/`DTT` > `EFF`/`IDD` > `SCR` > leaf assets). Opening a `.dat` or `.dtt` loads its paired sibling automatically and saves both on `Ctrl+S`.
- **WMB Studio (3D Viewport)**: Interactive Three.js viewport with game-specific shader previews for *Bayonetta 1*, *Bayonetta 2*, and *Bayonetta 3*, wireframe/normals/skeleton overlays, mesh picking, material parameter editing, and cross-game material conversion.
- **Texture Pipeline**: Native `DDS` (BC1-BC7), Nintendo Switch `XT1`/`BNTX` (Tegra X1 GOB swizzling and ASTC `4x4` through `12x12`), and Wii U `GTX` decoding and encoding, with alpha edge bleeding, mipmap generation, batch export/recompress/replace, and 3D cubemap preview.
- **UVD Atlas Editor**: Inspect UI sprite groups, crop individual sprites to PNG, add or replace sprites, and repack groups into power-of-two atlases.
- **BXM Tree Editor**: Interactive binary XML tree with inline editing, node creation/deletion, and raw XML view.
- **PKZ Archives**: Full directory packing, staged diff tracking with per-entry or full revert, multi-select bulk actions, and configurable compression (`None`, `ZStandard`, or `Oodle Kraken`) with an automatic size-savings threshold.
- **DAT Alignment and Texture Hashes**: Per-extension default alignment (`.dat` = `0x2000`; `.wmb`/`.wtb`/`.wtp`/`.exp`/`.eff`/`.sdx` = `0x1000`; `.wta`/`.sop`/`.bxm` = `0x40`; others = `0x10`), optional global custom alignment override, recursive batch DAT re-alignment, and right-click `CRC32 & 7F` texture hash validation and remapping for Bayonetta 2 and Bayonetta 3 models.

## CLI (`pgtools`)

Run `pgtools --help` or `pgtools <command> --help` for full flags.

### Archives

```bat
pgtools dat-extract <files...> [-o <output_dir>]
pgtools dat-create [input_dir] [-o <output.dat>] [--align [2048|0x800|default]]
pgtools dat-align <targets...> [--align <2048|0x800|default>] [--nested] [--per-type-default] [--no-recursive] [--no-pair-dtt] [-o <output>]
pgtools dat-ls <target> [--yaml]

pgtools pkz-extract <files...> [-o <output_dir>] [--oodle-dll <path>]
pgtools pkz-create <input_dir> -o <output.pkz> [--compression <none|zstd|oodle>] [--level <n>] [--force-compression] [--oodle-dll <path>]

pgtools eff-extract <file.eff> [-o <output_dir>]
pgtools eff-create <input_dir> [-o <output.eff>]
pgtools scr-extract <file.scr> [-o <output_dir>]
pgtools scr-create <input_dir> [-o <output.scr>]
```

### Textures and UI Atlases

```bat
pgtools wtx-extract <file.wtb|file.wta> [file.wtp] [-o <output_dir>] [--png]
pgtools wtx-create <input_dir> [-o <output.wtb>]
pgtools wtb-convert-wiiu-pc <file.wtb|file.wta> [-o <output.wtb>]
pgtools wtb-import-textures <target.wtb> <source.wtb|source.wta> [--overwrite]

pgtools uvd-info <file.uvd|file.dat>
pgtools uvd-list <file.uvd|file.dat> [-g <group>]
pgtools uvd-extract <file.uvd|file.dat> [-g <group>...] [-t <texture>...] [-o <output_dir>] [--json]
pgtools uvd-repack <group_dir> --uvd <file.uvd> [-g <group>] [-o <output.uvd>] [-t <sheet.png>] [--padding 4] [--max-size 4096]
```

### Models, XML, Animations, and Cloth

```bat
pgtools wmb-tool <input.wmb|input.wmb3> [options]
  --cleanup-vertexes          Remove unused vertices
  --cleanup-materials         Remove unused materials
  --cleanup-material-sizes    Compact material parameter blocks
  --maximize-material-sizes   Pad material parameter blocks to maximum size
  --remove-triangle-strips    Convert triangle strips to triangle lists
  --normals-to-wide           Convert compressed normals to wide format
  --check-normals             Validate vertex normals
  --swap-endian               Convert between Wii U (big-endian) and PC (little-endian)
  --scale <factor>            Scale model geometry
  --export-obj <out.obj>      Export geometry to Wavefront OBJ
  --dump-bones                Print bone table
  --dump-materials            Print material table
  --check-texture-hashes      Dry-run check for CRC32 & 7F texture hashes (B2/B3)
  --remap-texture-hashes      Remap invalid texture hashes across WMB and paired WTA/WTB

pgtools wmb-common-bones <files...>
pgtools wmb-get-bone-map <file.wmb|file.dat>
pgtools wmb-scene-dump <file.wmb|file.wmb3|file.dat> [--no-binary]
pgtools demo-model <b1|b2|b3> [-o <output.dat>]

pgtools bxm-decode <input.bxm> [-o <output.xml>]
pgtools bxm-encode <input.xml> [-o <output.bxm>] [--tag BXM]

pgtools clp-convert <input.clp> [-o <output>] [--swap-endian]
pgtools clh-convert <input.clh> [-o <output>] [--swap-endian]
pgtools clw-convert <input.clw> [-o <output>] [--swap-endian]
pgtools mot-tool <input.mot> [-o <output>] [--decode] [--decode-frame <n>] [--swap-endian] [--overwrite]
pgtools exp-tool <input.exp> [-o <output>] [--swap-endian] [--overwrite]
```

### Legacy Script Compatibility

`pgtools` supports busybox-style `argv[0]` dispatch. Symlinking or renaming copies of `pgtools.exe` to legacy `bayonetta_tools` names (such as `dat_extractor`, `dat_creator`, `dat_align`, `bxm_decoder`, or `wmb_cleanup`) routes arguments to the matching subcommand.

## Optional External Libraries and Settings

- **Oodle Kraken (`oo2core_*_win64.dll`)**: `ZStandard` and uncompressed `.pkz` archives work natively, and a pure-Rust Kraken fallback decompressor is built in. For faster *Bayonetta 3* / *Astral Chain* decompression and Kraken compression, place an `oo2core_*_win64.dll` next to the executables or configure its path in Studio settings.
- **NVIDIA Texture Tools (`nvtt*.dll`)**: Optional texture compression acceleration. Place next to the executable or configure in Studio settings.
- **Settings Location**: Preferences (theme, recent files, layout, DAT alignment, core DAT paths, and DLL paths) are saved to `%APPDATA%\pgtools-studio\pgtools_settings.json` on Windows (`$XDG_CONFIG_HOME/pgtools-studio/pgtools_settings.json` on Linux).

## Building from Source

### Prerequisites

- Rust toolchain (Edition 2021)
- Node.js and npm
- On Windows: Visual Studio Build Tools (MSVC) and Microsoft Edge WebView2 Runtime

### Workspace Structure

- `crates/pgtools-core`: Format parsers, encoders, texture codecs, scene builders, and archive logic
- `crates/pgtools-cli`: Unified CLI binary (`pgtools`)
- `crates/pgtools-gui`: React + TypeScript + Three.js frontend and Tauri v2 desktop shell (`pgtools-gui`)

### Build Commands

Windows release build (builds frontend, `PGTools-Studio.exe`, `pgtools.exe`, `release\`, `PGTools-Studio-Release.zip`, and `PGTools-Studio-Source.zip`):

```bat
.\publish_release.bat
```

`publish_release.bat` automatically copies any `oo2core*.dll` or `nvtt*.dll` present in the repository root into `release\`. For an explicit PowerShell staging script without bundling untracked DLLs by default:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\package-release.ps1 -Zip
```

Manual build:

```bat
cd crates\pgtools-gui
npm install
npm run build
cd ..\..
cargo build --release -p pgtools-gui --features tauri/custom-protocol
cargo build --release -p pgtools-cli
```

## Credits

Based on the format research and original Ruby toolkit [`bayonetta_tools`](https://github.com/Kerilk/bayonetta_tools) by Brice Videau.

## Disclaimer
This program includes ai-generated code.