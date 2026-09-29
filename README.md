# VTD Windows

Offline, multilingual voice dictation for Windows. A minimal fork of [MQ37/vtd](https://github.com/MQ37/vtd), built with Rust, whisper.cpp and Vulkan.

Runs in the background with a small tray icon and no main window or cloud service. Audio stays on your computer. The microphone is active only while recording.

## Use

Extract the package into a writable folder and run `vtd.exe`.

- **F8:** hold to record, release to finish.
- **F9:** press to start, press again to finish.
- **Esc:** cancel.

Click the mascot tray icon for **⏸ Pozastavit**, **▶ Spustit** or **✕ Ukončit**. Pausing stops recording, prevents pending text insertion and lets F8/F9 pass through to other applications. The tooltip shows the current state. The icon is a cropped version of the upstream mascot, embedded in the executable at multiple sizes.

Choose the destination text field before finishing. You can switch away and back while recording. Once you finish, keep focus in that field until the text appears. Changes during transcription block automatic insertion; `vtd copy` recovers the last transcript.

The tray starts without loading the model or Whisper libraries. Recording starts `vtd-engine.exe`, so model loading can overlap with speaking. Keep both executables in the same folder. After 30 seconds of inactivity, or when paused, that process exits and releases its memory and GPU resources. If transcription is already running, pausing discards its result and closes the process when it finishes. Recordings are not saved by default.

Windows supplies 16 kHz mono audio directly when supported, with the device's default format as a fallback. The recording buffer starts at 30 seconds and grows up to the configured limit. A new recording can start while the preceding transcription finishes; at most two transcriptions can be pending.

The package needs a Vulkan-capable GPU driver. No Rust, Python or Vulkan SDK installation is needed to run it. The x64 build targets discrete GPUs and AMD Strix Halo; Strix Halo still needs testing on that hardware.

If the model is missing, run this from the extracted package:

```powershell
powershell -ExecutionPolicy Bypass -File .\download-model.ps1 -Destination .\models
```

## Setup, autostart and removal

Share the complete ZIP rather than just `vtd.exe`. The default ZIP excludes the model;
setup downloads the Q5 model automatically (about 574 MB).
Extract it into a writable folder where you intend to keep it. In PowerShell opened
in that folder, run:

```powershell
powershell -ExecutionPolicy Bypass -File .\Install.ps1
```

This downloads the default Q5 model if missing, verifies the download with SHA256,
starts VTD and enables startup at Windows sign-in
for the current user. It uses the extracted folder directly, without administrator
rights or an entry in Installed apps. Running `vtd.exe` alone does not enable autostart.
Do not move the folder afterwards; if you do, rerun `Install.ps1` from its new location
after exiting VTD.
The first download requires internet; dictation then works offline. An existing model
is reused. If downloading fails, setup stops before starting VTD or enabling autostart;
run it again to retry. A missing custom model must be supplied manually.

To change autostart without starting or stopping VTD:

```powershell
powershell -ExecutionPolicy Bypass -File .\Autostart.ps1 -Mode On
powershell -ExecutionPolicy Bypass -File .\Autostart.ps1 -Mode Off
```

`Autostart.ps1` defaults to `On`. To remove autostart and close VTD from this folder:

```powershell
powershell -ExecutionPolicy Bypass -File .\Uninstall.ps1
```

Then delete the folder yourself, including its model and configuration. The script
keeps those files and can also be run when VTD is already stopped or autostart is disabled.
These scripts manage the same per-user autostart entry as `vtd autostart on|off`.

## Configuration

Edit `vtd.json` beside the executable, then restart VTD.

| Setting | Default | Purpose |
| --- | --- | --- |
| `language` | `cs` | Language code, e.g. `cs`, `en`, `de`; `auto` detects the language |
| `microphone` | `null` | System default, or an exact name from `vtd devices` |
| `gpu` | `null` | Prefer discrete GPU, otherwise first available; or choose its index |
| `clipboard_paste` | `false` | Paste using Ctrl+V instead of typing Unicode characters |
| `trigger_key` | `119` | Hold-to-record key, F8 |
| `toggle_key` | `120` | Start/stop key, F9; must differ from `trigger_key` |
| `toggle` | `false` | Also make `trigger_key` a start/stop key |
| `idle_unload_seconds` | `30` | Close the idle transcription process; `0` keeps it loaded |
| `max_recording_seconds` | `300` | Recording limit, up to 300 seconds |
| `silence_rms` | `0.002` | Silence threshold |
| `filter_subtitle_credits` | `true` | Remove the known trailing JohnyX subtitle credit; disable to dictate that phrase literally |
| `threads` | `4` | CPU worker threads |
| `model` | `models/ggml-large-v3-turbo-q5_0.bin` | Model path, relative to the configuration or absolute |

The model supports multiple languages; `cs` is only the initial configuration. Set a language explicitly for predictable short dictation, or use `auto`.

The full recording is transcribed without trimming silence. An energy check skips silent recordings. Whisper can still invent text in noise. The optional subtitle filter only removes the known final “Titulky vytvořil JohnyX” credit and spelling variants; it does not remove ordinary closing sentences.

Enable `clipboard_paste` if an editor drops or repeats typed characters. This replaces the clipboard with the latest transcript. Applications running as administrator may reject insertion from VTD running without elevation.

## Commands

```text
vtd status
vtd stop
vtd pause
vtd resume
vtd copy
vtd devices
vtd autostart on|off
vtd transcribe recording.wav [REPEATS]
vtd run --capture-next test.wav
```

Autostart is opt-in. `--capture-next` explicitly saves only the next completed recording locally, without overwriting an existing file. Normal dictation logs timing and microphone diagnostics, not transcripts.

## Build

Requires Rust stable, Visual Studio C++ Build Tools with Windows SDK, CMake/Ninja, Vulkan SDK and libclang. Setup also needs Python and 7-Zip.

```powershell
.\scripts\setup-windows.ps1
.\scripts\build-windows.ps1
.\scripts\build-windows.ps1 -Test
.\scripts\package-windows.ps1
```

The build uses `C:\vtd-build` to avoid Windows path-length limits. Override it with `-BuildDir`. CPU-specific native optimizations are disabled for portability; Vulkan selects the GPU at runtime.

The Windows release script uses full Rust LTO, one code generation unit and abort-on-panic, with symbols stripped. The tray uses size optimization; the engine retains speed optimization. Native C/C++ functions and data are individually removable by the linker. Cargo and CMake run at most two top-level build jobs; each shader generator compiles one shader at a time. Packages include only the three Visual C++ runtime DLLs required by the executables.

Vulkan shaders are embedded with lossless XPRESS Huffman compression using the Windows API. Only shaders needed by the selected GPU are decompressed, directly into temporary memory. Whisper, Vulkan and compression live in the separate engine executable; the tray does not load them. GPU calculations and supported shader variants are unchanged. The native patch stores vocabulary text once, uses a sorted token-ID index, caches suppression IDs, and releases shader modules after pipeline creation. It is applied to Cargo's build copy; changed patches invalidate the native build automatically.

Packaging excludes models by default, even when an older package folder contains one.
For an offline installation package, first run `scripts/download-model.ps1`, then
`scripts/package-windows.ps1 -WithModel` to include Q5. Local packaging shares that
model through a hard link when supported. ZIP creation streams files without buffering
the complete model in managed memory.

Fresh setup extracts only the Vulkan compiler, headers and import library needed by the build. The installer is processed in 1 MiB blocks without copying it into the Python heap; completed setup is reused on subsequent runs.

The original Linux source is retained. See [upstream](https://github.com/MQ37/vtd) for Linux instructions. Windows code lives in `src/windows`.

To regenerate the icon, install `svgo@4.1.0` and `@resvg/resvg-js@2.6.2` with `npm install --prefix .tools/icon-tools`, put oxipng on PATH (or set `OXIPNG`), and run `node scripts/build-icon.mjs`. It optimizes the SVG and renders each existing ICO size directly from the vector. The 16px tray image uses a native 32-bit DIB to avoid loading the Windows PNG decoder at startup; larger images use PNG with `oxipng -o max -Z`. Pixel data and all sizes are preserved. These tools are not required for normal builds.
