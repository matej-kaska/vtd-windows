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

The model stays loaded for five minutes after use, then releases memory. The next dictation reloads it. Recordings are not saved by default.

The package needs a Vulkan-capable GPU driver. No Rust, Python or Vulkan SDK installation is needed to run it. The x64 build targets discrete GPUs and AMD Strix Halo; Strix Halo still needs testing on that hardware.

If the model is missing, run this from the extracted package:

```powershell
powershell -ExecutionPolicy Bypass -File .\download-model.ps1 -Destination .\models
```

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
| `idle_unload_seconds` | `300` | Unload the idle model; `0` keeps it loaded |
| `max_recording_seconds` | `120` | Recording limit, up to 300 seconds |
| `silence_rms` | `0.002` | Silence threshold |
| `threads` | `4` | CPU worker threads |
| `model` | `models/ggml-large-v3-turbo-q5_0.bin` | Model path, relative to the configuration or absolute |

The model supports multiple languages; `cs` is only the initial configuration. Set a language explicitly for predictable short dictation, or use `auto`.

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
.\scripts\download-model.ps1
.\scripts\package-windows.ps1 -WithModel
```

The build uses `C:\vtd-build` to avoid Windows path-length limits. Override it with `-BuildDir`. CPU-specific native optimizations are disabled for portability; Vulkan selects the GPU at runtime.

The original Linux source is retained. See [upstream](https://github.com/MQ37/vtd) for Linux instructions. Windows code lives in `src/windows`.
