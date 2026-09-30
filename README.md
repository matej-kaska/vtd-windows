# VTD Windows

**Offline voice dictation. GPU accelerated. Exceptionally light when idle.**

Speak into a text field using a keyboard shortcut. VTD turns your speech into text locally with Whisper large-v3-turbo, then inserts the result. It lives in the Windows system tray, with a small native settings window when you need it.

**Around 1 MiB of private RAM for the idle tray · About 6.9 MB for the app ZIP · Automatic model unloading**

[Download](https://github.com/matej-kaska/vtd-windows/releases) · [Get started](#get-started) · [Settings](#settings) · [Resource-usage measurements](#resource-usage)

- **Local and multilingual.** Audio stays on your computer. Czech, English and other Whisper languages are supported, including automatic language detection.
- **Two ways to record.** Hold a key, or press once to start and again to finish. The microphone is active only during recording.
- **Fast insertion with clipboard restoration.** Keep copied text, screenshots and files while inserting your dictation.
- **Recover the last transcript.** Missed the text field? Click it and press F10 to insert the last completed result.
- **Small by design.** Native Rust and Win32, a separate on-demand speech engine, and no bundled browser or GUI framework.

## Get started

You need **64-bit Windows and a GPU with a working Vulkan driver**. Rust, Python, CUDA and the Vulkan SDK are not required to run the package.

1. Download **`VTD-Setup.exe`** from [Releases](https://github.com/matej-kaska/vtd-windows/releases).
2. Choose your shortcuts, speech language, playback muting and startup preference.
3. Download the recommended model, enter a custom HTTPS model link, or select an existing `.bin` model.

The native installer is about **120 KB**. It installs for your Windows account without administrator rights, PowerShell commands or a separate installer runtime. It downloads the matching application package and verifies its SHA-256 before installing. VTD appears in Start and in Windows Installed apps.

The recommended **Whisper large-v3-turbo Q5 model is about 574 MB**, downloaded separately and checked against its known SHA-256. Custom URLs accept an optional SHA-256; all models must use the whisper.cpp GGML format. Existing model files are used in place. Internet is needed for initial downloads; dictation works offline afterwards. Put the matching `vtd-runtime-x64.zip` beside the installer and select an existing model to install offline.

The initial installer release is unsigned; code signing is planned separately. For local builds and release automation, see [installer documentation](installer/README.md).

<details>
<summary>Portable ZIP installation</summary>

Download `vtd-windows-x64.zip`, extract it into a writable folder where you intend to keep VTD, then run:

```powershell
powershell -ExecutionPolicy Bypass -File .\Install.ps1
```

The portable setup script downloads and verifies the default Q5 model, starts VTD and enables startup at Windows sign-in for your account. Existing configuration and model files are reused.

The extracted folder is the installation: no administrator rights, extra application copy or Installed apps entry. Keep both `vtd.exe` and `vtd-engine.exe` together. If you move the folder, exit VTD and run `Install.ps1` again from the new location. Running `vtd.exe` alone starts VTD without enabling autostart; the model must already be available.

</details>

## Dictate

Click the destination text field, record, then keep it focused until the text appears.

| Default key | Action |
| --- | --- |
| **F8** | Hold to record; release to transcribe and insert. |
| **F9** | Press to start; press again to transcribe and insert. |
| **F10** | Insert the last completed non-empty transcript again. |
| **Esc** | Cancel the current recording. |

Recordings can be up to **five minutes** long. You can start another recording while the previous one is being transcribed. F10 always uses the last *completed* result, which stays in RAM until VTD exits, even after the model unloads.

Switching away and back while recording is supported. Changing focus after finishing can prevent automatic insertion; click the intended field and use F10. Applications running as administrator may reject input from an unelevated VTD.

The tray menu provides **Pause**, **Resume**, **Settings...** and **Exit**. Pause stops recording, prevents pending insertion and lets the dictation keys pass through to other apps. The tray tooltip shows VTD's current state.

### Your clipboard

By default, VTD temporarily backs up the clipboard in RAM, pastes the transcript and restores the original contents. This supports common text, HTML, bitmap/PNG screenshots and copied file paths. If you copy something new during insertion, the new contents take priority. Unsupported private clipboard formats block insertion rather than discard the original data.

Restoration normally follows the target application's clipboard read, with a two-second fallback. Editors using unusual clipboard handling may need compatibility testing. The separate `vtd copy` command intentionally replaces the clipboard with the last transcript.

## Settings

Open **Settings...** from the tray when recording and transcription have finished.

| Setting | What you can change |
| --- | --- |
| Keyboard shortcuts | Choose a separate **F1-F24** key for holding to record, toggling recording and inserting the last transcript. |
| Speech language | Choose a language explicitly, or use automatic detection. |
| Start when I sign in to Windows | Enable or disable autostart for your account. |
| Mute playback while recording | Temporarily mute the default playback device while you speak. |

**Save** applies these changes immediately. **Cancel** discards them. Dictation shortcuts are paused while settings are open. The settings process exits when closed, releasing its UI memory.

On first setup, the speech language follows your Windows display language: Czech Windows selects `cs`, English selects `en`, and unsupported languages fall back to `auto`. Your saved choice is kept on later starts. Selecting a specific language can make short dictation more predictable. The interface itself is in English.

Playback muting is off by default. When enabled, VTD restores the device's original mute state after recording, cancellation or normal exit; an already-muted device stays muted. It does not change volume levels or microphone volume. If playback devices change mid-recording, VTD restores the original device.

## Resource usage

**The tray stays small; the speech engine runs only when needed.** Starting VTD does not load Whisper or the model. Starting a recording launches the engine so loading can overlap with speaking. By default, the engine exits after **30 seconds without work**, releasing its process memory and GPU allocations.

The figures below describe the **local release build measured on 30 September 2026**, with settings closed. Published ZIPs may contain an earlier build.

| Component or state | Measured usage |
| --- | --- |
| Idle tray | **0.7-1.5 MiB private resident RAM**; **6.6-13.1 MiB** including shared pages. |
| Engine after loading, repeated starts | **36.6-38.3 MiB private resident RAM**. First start of the new executable measured **61.7 MiB**. |
| Engine after a short transcript | **About 39.6 MiB private resident RAM**, held until the idle timeout. |
| Engine after a five-minute transcript | **About 50-53 MiB private resident RAM** across the long and following short test. |
| Peak during the five-minute test | **105.7 MiB full working set** on a repeated start; **150.9 MiB** on the first start. |
| Dedicated GPU memory | **About 948 MiB** with the Q5 model in an earlier GPU measurement on the same hardware. |
| Idle CPU and process I/O | **No increase in CPU time or read/write bytes** during the sampled idle intervals. |
| Download and disk space | **About 6.9 MB ZIP / 9.9 MB extracted**, plus the separate **574 MB model**. |

Private resident RAM counts pages unique to a process. The full working set also includes shared Windows and driver pages, so these are different numbers; Task Manager views may show different metrics. RAM uses **MiB** (1,048,576 bytes); download and file sizes use **MB** (1,000,000 bytes).

### What is allocated, and when is it released?

1. **Idle:** only the tray remains. Windows audio/COM resources can leave a little more RAM resident after microphone use; the recording buffer is not kept for F10.
2. **Recording:** the audio buffer starts with 30 seconds of capacity and grows up to the five-minute limit. At mono 16 kHz float audio, that is **1.83-18.31 MiB of capacity**, not necessarily all resident immediately. A microphone using a higher sample rate needs more space.
3. **Transcribing:** the tray releases its audio after sending it to the engine. The engine releases the input after preparing the spectrogram, then releases temporary spectrogram data after decoding. The model, decoder state and driver still require CPU RAM alongside GPU memory.
4. **After 30 idle seconds:** the entire engine process exits. Only the lightweight tray and the last transcript remain. Set a different timeout in the configuration if desired.

Normal dictation does **not** save WAV recordings or a transcript history to disk. Loading the model reads its file, and Windows or the GPU driver may use caching or paging. Idle process-I/O measurements do not mean that the physical disk can never be active.

<details>
<summary>Measurement conditions and speed</summary>

Measured on **Ryzen 7 7800X3D, Radeon RX 7800 XT and 32 GB RAM**, using Whisper large-v3-turbo Q5, Vulkan, four CPU worker threads and beam size 5. RAM varies with the driver, cache state, recording length and clipboard contents. The GPU-memory figure comes from the preceding audit of the same model and hardware; it was not remeasured after the final CPU-memory changes.

A 14.1-second Czech fixture took approximately **0.8 seconds to transcribe in a warmed run** with automatic language detection. A 300-second fixture took approximately **9.9 seconds**. These measure submission-to-result time with an already-loaded engine, excluding microphone capture and insertion. The long fixture repeats the short recording; it is a throughput check, not a five-minute conversation or accuracy benchmark. First use also needs model loading and GPU initialization.

The idle lifecycle test observed the worker exiting **30.046 seconds** after returning a result. No forced working-set trimming was used. First-start measurements did not flush Windows or GPU caches.

The final optimization round reduced the engine executable from **15.29 MB to 8.47 MB** and the model-free ZIP from **12.86 MB to about 6.94 MB**. The current tray executable, including the shortcut reliability fix, is **0.65 MB**. The Q5 model and decoding parameters stayed the same; all 20 production-path comparison transcripts and eight additional fixed-Czech results matched their reference outputs exactly. This verifies the tested fixtures, not universal recognition accuracy.

</details>

## Advanced configuration

For microphone selection, GPU choice, memory timeout and other options, edit **`vtd.json` beside `vtd.exe`**, then restart VTD. The file is created on first setup or launch. Relative model paths are resolved from the configuration's folder.

<details>
<summary>All configuration options and defaults</summary>

| Option | Default | Meaning |
| --- | --- | --- |
| `language` | Windows display language | A Whisper language code such as `cs`, `en` or `de`; `auto` detects it. Chosen once when the file is created. |
| `microphone` | `null` | System default, or an exact device name from `vtd devices`. |
| `gpu` | `null` | Prefer a discrete GPU, otherwise the first available GPU; set an index from `vtd devices` to choose explicitly. |
| `trigger_key` | `119` | F8: hold to record. |
| `toggle_key` | `120` | F9: start/stop recording. |
| `replay_key` | `121` | F10: insert the last transcript. All three keys must be distinct; F1-F24 map to 112-135. |
| `toggle` | `false` | Make `trigger_key` toggle recording too, instead of requiring a hold. |
| `clipboard_paste` | `true` | Fast paste with clipboard restoration. `false` uses slower Unicode typing for automatic insertion; F10 still uses fast paste. |
| `mute_output` | `false` | Mute playback only during recording. |
| `idle_unload_seconds` | `30` | Exit the idle engine after this many seconds; `0` keeps it loaded. |
| `max_recording_seconds` | `300` | Recording limit in seconds, from 1 to 300. |
| `silence_rms` | `0.002` | Energy threshold for skipping a whole silent recording; valid range 0-0.1. |
| `filter_subtitle_credits` | `true` | Remove the known trailing JohnyX subtitle credit and spelling variants. |
| `threads` | `4` | CPU worker threads, from 1 to 16. GPU acceleration still uses some CPU work. |
| `model` | `models/ggml-large-v3-turbo-q5_0.bin` | Relative or absolute path to a multilingual whisper.cpp model. |

Speech recordings are processed without trimming silence from their beginnings or ends. Very short or silent recordings are skipped. Whisper may invent words in noise; the subtitle filter targets the known trailing "Titulky vytvořil JohnyX" credit, not ordinary closing sentences. Disable it if you need to dictate that phrase literally.

Autostart is a Windows per-user setting, managed through Settings or the commands below; it is not a `vtd.json` field.

</details>

## Autostart and removal

Use **Settings...** to change autostart, or run one of these commands from the application folder:

```powershell
.\vtd.exe autostart on
.\vtd.exe autostart off
```

For an installed copy, uninstall **VTD** through **Windows Settings > Apps > Installed apps**, or run `Uninstall.exe` in its folder. Models and settings are kept by default. The optional cleanup removes only settings and the two model filenames managed by the installer; external models and unrelated files are kept.

For a portable ZIP, remove autostart and stop that installation with:

```powershell
powershell -ExecutionPolicy Bypass -File .\Uninstall.ps1
```

The script keeps your files. Delete the extracted folder yourself to remove the program, model and configuration. The included `Autostart.ps1 -Mode On` / `-Mode Off` script manages the same startup entry.

<details>
<summary>More command-line controls and diagnostics</summary>

Run these from the extracted folder:

```powershell
.\vtd.exe status
.\vtd.exe stop
.\vtd.exe pause
.\vtd.exe resume
.\vtd.exe settings
.\vtd.exe copy
.\vtd.exe devices
.\vtd.exe transcribe recording.wav
```

`copy` deliberately places the last transcript on the clipboard. `transcribe` accepts an optional repeat count from 1 to 20 after the WAV path.

To explicitly save just the next completed recording locally, exit the running tray first, then run:

```powershell
.\vtd.exe run --capture-next test.wav
```

An existing WAV is never overwritten. Normal recording saves no audio file.

To download or verify the default model manually:

```powershell
powershell -ExecutionPolicy Bypass -File .\download-model.ps1 -Destination .\models
```

</details>

## Build from source

<details>
<summary>Windows build, packaging and icon tools</summary>

Requires Rust stable, Visual Studio C++ Build Tools with Windows SDK, CMake/Ninja, Vulkan SDK and libclang. The setup script supplies local Vulkan build tools and libclang; it also needs Python and 7-Zip.

```powershell
.\scripts\setup-windows.ps1
.\scripts\build-windows.ps1
.\scripts\build-windows.ps1 -Test
.\scripts\package-windows.ps1
.\scripts\build-installer.ps1
.\scripts\test-installer.ps1
```

Builds use `C:\vtd-build` to avoid Windows path-length limits; override with `-BuildDir`. CPU-specific native optimizations are disabled for portability. Vulkan chooses the GPU at runtime; there is no automatic CPU-only fallback.

Release builds use full Rust LTO, one code generation unit, stripped symbols and abort-on-panic. The tray favors size; the engine retains speed optimization. Native patches remove unused model operations, reduce temporary allocations and compress embedded Vulkan shaders losslessly. Only needed shaders are decompressed. Patches apply to Cargo's build copy and changes invalidate the native build automatically.

The default ZIP excludes models, personal configuration and development tools. `scripts\package-windows.ps1 -WithModel` includes the default Q5 model after it has been downloaded. Use `-OutputDir` with a fresh staging folder when the normal package folder contains a running executable. Packaging uses single-threaded maximum Deflate when 7-Zip is available, otherwise .NET compression; both produce a standard Windows-compatible ZIP.

The installer build downloads pinned, hash-checked NSIS tools into `.tools/installer`, then produces `dist/installer/VTD-Setup.exe` and its matching runtime ZIP. The installer test suite uses isolated directories and registry entries. Add `-RealModelDownload` to test the real 574 MB HTTPS model download instead of a local fixture. The [release workflow](.github/workflows/release.yml) builds, tests and publishes matching assets when a version tag is pushed; see [release instructions](installer/README.md#releases).

For icon regeneration, install `svgo@4.1.0` and `@resvg/resvg-js@2.6.2` using `npm install --prefix .tools/icon-tools`, put oxipng on PATH or set `OXIPNG`, then run `node scripts/build-icon.mjs`. Each size is rendered directly from the optimized SVG. The 16px tray image uses a native DIB to avoid loading a PNG decoder at startup; larger sizes use optimized PNG. These tools are unnecessary for normal builds.

</details>

## Platforms and credits

This fork targets **Windows x64 with Vulkan** and has been tested on the Radeon RX 7800 XT. AMD Strix Halo is an intended target for the same build, but still needs testing on physical hardware. Its shared-memory usage will differ from a discrete GPU.

Apple Silicon macOS support is planned, including **Metal and Core ML / Apple Neural Engine** acceleration. There is no macOS build yet.

Based on [MQ37/vtd](https://github.com/MQ37/vtd), with its mascot adapted for the Windows tray. Speech recognition uses [whisper.cpp](https://github.com/ggml-org/whisper.cpp). The original Linux source is retained; use the upstream project for Linux instructions.

Released under the [Unlicense](LICENSE). Bundled components retain their own licenses; see [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt).
