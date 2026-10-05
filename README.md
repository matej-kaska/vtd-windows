<p align="center">
  <a href="https://github.com/MQ37/vtd">
    <img src="https://raw.githubusercontent.com/MQ37/vtd/master/logo.svg" width="160" height="160" alt="VTD mascot: a little red demon with golden horns and a spade-tipped tail">
  </a>
</p>

<h1 align="center">VTD Windows</h1>

<p align="center"><strong>Leave it on. Dictate whenever you need it.</strong></p>

**Dictation you can leave running all day.** Start VTD with Windows and forget about it until you need to type. Hold **F8** or tap **F9**, speak, and your words go straight into the active text field. VTD is built to stay ready in the tray with a tiny idle footprint.

**About 0.1 MB of RAM while idle.** After a fresh restart, the tray measured **108 KiB (~0.11 MiB) of private resident RAM**, with **zero measured CPU time and disk I/O** over 30 seconds. The speech engine loads when you record and exits after 30 idle seconds, releasing its RAM and VRAM. [Measured footprint and limits](docs/tray-memory.md#fresh-restart-v042).

Choose **Canary, Redux or Whisper** for GPU accelerated dictation. Audio stays on your computer; no account or cloud transcription service is needed. The microphone opens only when you start recording.

[**Windows installer**](https://github.com/matej-kaska/vtd-windows/releases/latest/download/VTD-Setup.exe) · [Portable ZIP](https://github.com/matej-kaska/vtd-windows/releases/latest/download/vtd-windows-x64.zip) · [Models](#choose-a-model) · [Settings](#settings) · [RAM and VRAM](#resource-usage)

- **Made to stay on**, with Windows startup, about 0.1 MB idle private RAM and recording on demand.
- **AMD GPU acceleration through Vulkan**, tested on Radeon RX 7800 XT.
- **Three downloadable models**, with speed, peak VRAM and Czech accuracy shown in setup and Settings.
- **Hold or toggle to record**, with customizable shortcuts and replay of the last transcript.
- **Clipboard restoration** after insertion, including common text, screenshots and copied file paths.
- **Automatic model unloading** after 30 idle seconds, leaving only the tray process.
- **Native Rust and Win32 UI**, without a bundled browser or GUI framework.

**v0.4.3 replaces the Parakeet preset with Redux.** Existing installations keep their selected model and preferences; older Parakeet files remain supported as custom models.

## Get started

Requires **Windows 10/11 x64, an AVX2-capable CPU and a GPU with a working Vulkan driver**. CUDA, Python, Rust and the Vulkan SDK are not required to run VTD. Inference does not silently fall back to the CPU if GPU initialization fails.

1. Download and open [**VTD-Setup.exe**](https://github.com/matej-kaska/vtd-windows/releases/latest/download/VTD-Setup.exe).
2. Choose a model and speech language. **Canary with Czech selected** is the recommended starting point for Czech dictation.
3. Set your shortcuts, startup preference and optional playback muting, then finish installation.
4. Click a text field and press **F9** to start recording. Press **F9** again to transcribe and insert.

Setup installs for your Windows account into `%LOCALAPPDATA%\Programs\VTD` by default, without administrator rights. The installer is about **120 KB**; it downloads the matching application package and chosen model separately, verifying their SHA-256 checksums. Internet access is needed for downloads, then dictation works offline. Releases are currently unsigned.

Setup also accepts an existing model or a custom HTTPS URL. Supported formats are Whisper GGML `.bin` and Canary/Parakeet GGUF `.gguf`; other model architectures are not supported. For offline installation, put the matching `vtd-runtime-x64.zip` beside the installer and select an existing model.

<details>
<summary>Portable installation</summary>

Extract [**vtd-windows-x64.zip**](https://github.com/matej-kaska/vtd-windows/releases/latest/download/vtd-windows-x64.zip) into a writable folder, then run:

```powershell
powershell -ExecutionPolicy Bypass -File .\Install.ps1
```

The script downloads and verifies the configured preset, starts VTD and enables startup at sign-in. New configurations choose Canary for a supported Windows display language, otherwise Whisper. Existing preferences and models are reused.

The extracted folder is the installation. Keep `vtd.exe`, `vtd-helper.exe`, `vtd-engine.exe`, `vtd-transcribe.exe` and the included DLLs together. If you move the folder, exit VTD and run `Install.ps1` from its new location. Running `vtd.exe` directly starts the tray without changing autostart.

</details>

## Choose a model

All three presets use the selected Vulkan GPU. Canary offers the lowest Czech word error rate in our reference comparison; Redux offers a smaller download and uses less VRAM. Whisper remains available with its separate optimized whisper.cpp engine.

| Preset | Warm 30 s audio | Peak dedicated VRAM | Czech WER ↓ | Download |
| --- | ---: | ---: | ---: | ---: |
| **Canary-1B-v2 Q4_K_M** | 0.34 s | 1,334 MiB | **9.78%** | 735 MB |
| **Redux** | **0.21 s** | **563 MiB** | 12.32% | **159 MB** |
| **Whisper large-v3-turbo Q5_0** | 0.66 s | 948 MiB | 12.22% | 574 MB |

These are the reference results shown in the model picker: **RX 7800 XT / Ryzen 7 7800X3D**, four CPU threads, Czech selected, and 100 deterministic Czech FLEURS test clips with 1,964 reference words. Lower WER is better. Timing excludes model loading; VRAM includes loading and transcription. Results depend on the PC and recording. Q4_K_M was also compared with Q5_K_M and Q8_0 for Canary and Parakeet. [Benchmark and optimization details](docs/model-engine.md).

**Language selection matters:** Canary requires an explicit supported language. Redux supports automatic detection across its 25 supported languages. Whisper provides the widest language selection and automatic detection. Settings checks compatibility before applying a change.

Redux replaces the original Parakeet preset and uses the fast Q4 Vulkan runtime.
Its 159 MB ternary download expands losslessly for GPU inference; download size
is not VRAM usage. Redux was measured in the integrated engine on 4 October;
the other rows retain their earlier reference measurements. Background CPU
load makes small timing differences inconclusive. [Redux comparison and integration](docs/redux-benchmark.md).
Old Parakeet files still work as custom models and are not automatically deleted.

Long Canary/Redux recordings are decoded in chunks of at most 30 seconds, preferring quiet boundaries. Every audio sample is retained, but words crossing a forced split can still be affected. A result reported as truncated by the native decoder is rejected instead of inserted.

## Dictate

Keep the destination text field focused until the transcript appears.

| Default key | Action |
| --- | --- |
| **F8** | Hold to record; release to transcribe and insert. |
| **F9** | Press to start; press again to finish. |
| **F10** | Insert the last completed non-empty transcript again. |
| **Esc** | Cancel the current recording. |

Recordings can be up to **five minutes** long. You can record again while the previous recording is being transcribed. F10 uses the last completed result, which stays in RAM until VTD exits, including after model unloading.

Changing focus after finishing can prevent automatic insertion; click the intended field and use F10. Elevated applications may reject input from an unelevated VTD. **Pause** in the tray menu stops recording, prevents pending insertion and lets the shortcuts pass through to other apps.

VTD temporarily backs up the clipboard in RAM, pastes the transcript and restores the original contents. Common text, HTML, bitmap/PNG screenshots and copied file paths are supported. Newly copied content takes priority over restoration. Unsupported private clipboard formats block insertion to preserve the original data. Restoration follows the destination's clipboard read, with a two-second fallback; unusual clipboard handling may require compatibility testing.

## Settings

Open **Settings...** from the tray when recording and transcription have finished. Dictation shortcuts are paused while Settings is open.

| Control | What it does |
| --- | --- |
| Model selection | Shows each preset's name, reference speed, peak VRAM, Czech WER and download state. |
| **Apply / Save** | Downloads and verifies a missing selected model, then switches to it. Apply keeps Settings open; Save closes it. |
| **Download / Stop download** | Downloads a preset without switching models, or cancels the transfer and removes the partial file. |
| **Delete...** | Removes an unused downloaded preset after confirmation. Apply another model before deleting the one marked **In use**. External model files are kept. |
| Keyboard shortcuts | Click a field and press a key or combination. The three actions must use different shortcuts. |
| Speech language | Select a language supported by the chosen model. |
| Start when I sign in | Controls Windows autostart for this installation. |
| Mute playback while recording | Temporarily mutes playback and restores the device's previous mute state afterwards. |

**Cancel** discards unsaved preferences. Completed downloads, deletions and changes already applied are kept. Changing models unloads the previous engine. Closing Settings exits its separate UI process.

New configurations use the Windows display language; upgrades preserve the saved choice. The interface is in English. Shortcut fields reserve **Space, Tab, Enter and Esc**. On a laptop, press the desired top-row key without Fn: if Windows receives a media key, VTD can save that key. Firmware-only keys require Fn Lock or another shortcut.

## Resource usage

**Starting the tray does not load a model.** The native tray owns the shortcuts and last transcript; settings and recording run in temporary helper processes. Recording launches the appropriate engine so loading can overlap with speaking. After **30 seconds without work**, the engine and recording helper exit and release their process memory and GPU allocations. The timeout is configurable.

After a fresh restart, the v0.4.2 tray measured **108 KiB private resident RAM (~0.11 MiB)**, **264 KiB total resident RAM including shared pages** and **1,020 KiB private committed memory**, with no model, menu or Settings open. Over the 30-second idle check it used **zero measured CPU time and disk I/O**. These are local observations, not fixed memory limits. See [tray footprint and verification](docs/tray-memory.md#fresh-restart-v042) for methodology, comparisons and limitations.

Historical optimized worker measurements before Redux integration:

| Model | Peak resident RAM, 30 s audio | Peak resident RAM, 5 min audio |
| --- | ---: | ---: |
| Canary Q4_K_M | **95.5–95.7 MiB** | **114.3–114.9 MiB** |
| Legacy Parakeet Q4_K_M | **96.0–96.1 MiB** | **112.6–112.7 MiB** |

These are **whole-process working-set peaks**, including shared pages, on RX 7800 XT / Ryzen 7 7800X3D with four threads. They exclude the separate tray and do not represent VRAM or private commit. The five-minute fixture repeats a short recording; it is a throughput check, not a five-minute conversation. Driver state, system load and memory pressure affect the measurements.

The latest allocator change saved another **1.2–1.45 MiB** of Canary's 30-second peak RAM without increasing sampled peak VRAM. All **100 Canary and 100 Parakeet Czech transcripts** remained identical to the preceding engine. A separate alternating timing check measured **0.319 s before / 0.324 s after** for 30-second Canary audio; this change does not establish a speed improvement. Private commit does not consistently fall. [Exact comparisons, tests and measurement limits](docs/model-engine.md#packed-graph-allocation-records-1-october-2026).

The model-free application download is about **19 MB**, plus the selected model. RAM/VRAM figures use **MiB** (1,048,576 bytes); file downloads use **MB** (1,000,000 bytes).

Normal dictation saves neither WAV recordings nor transcript history to disk. Audio and the last transcript live in RAM. Model loading reads the model file; Windows and GPU drivers may use caching or paging.

## Configuration and commands

Most preferences are available in Settings. For microphone selection, GPU index, idle timeout and recording limits, edit **`vtd.json` beside `vtd.exe`** and restart VTD. Relative model paths resolve from that folder. Personal configuration is not distributed in the package.

<details>
<summary>All configuration options and defaults</summary>

| Option | Default | Meaning |
| --- | --- | --- |
| `language` | Windows display language | A supported language code such as `cs`, `en` or `de`; `auto` is available for Redux and Whisper. Chosen once when the file is created. |
| `microphone` | `null` | System default, or an exact device name from `vtd devices`. |
| `gpu` | `null` | Prefer a discrete GPU, otherwise the first available GPU; set an index from `vtd devices` to choose explicitly. |
| `trigger_key` | `119` | F8: hold to record. |
| `toggle_key` | `120` | F9: start/stop recording. |
| `replay_key` | `121` | F10: insert the last transcript. All three shortcuts must be distinct. Keys use Windows virtual-key codes; add 256 for Shift, 512 for Ctrl and 1024 for Alt (Ctrl+Shift+F9 = 888). |
| `toggle` | `false` | Make `trigger_key` toggle recording too, instead of requiring a hold. |
| `clipboard_paste` | `true` | Fast paste with clipboard restoration. `false` uses slower Unicode typing for automatic insertion; F10 still uses fast paste. |
| `mute_output` | `false` | Mute playback only during recording. |
| `idle_unload_seconds` | `30` | Exit the idle engine after this many seconds; `0` keeps it loaded. |
| `max_recording_seconds` | `300` | Recording limit in seconds, from 1 to 300. |
| `silence_rms` | `0.002` | Energy threshold for skipping a whole silent recording; valid range 0-0.1. |
| `filter_subtitle_credits` | `true` | Remove the known trailing JohnyX subtitle credit and spelling variants. |
| `threads` | `4` | CPU worker threads, from 1 to 16. GPU acceleration still uses some CPU work. |
| `model` | Canary Q4_K_M for supported Windows languages; otherwise Whisper Q5_0 | Relative or absolute path to a supported GGML/GGUF model. Older configurations omitting this field retain their Whisper default. |

Speech recordings are processed without trimming silence from their beginnings or ends. Very short or silent recordings are skipped. Whisper may invent words in noise; the subtitle filter targets the known trailing "Titulky vytvořil JohnyX" credit, not ordinary closing sentences. Disable it if you need to dictate that phrase literally.

Autostart is a Windows per-user setting, managed through Settings or the commands below; it is not a `vtd.json` field.

</details>

```powershell
.\vtd.exe settings
.\vtd.exe status
.\vtd.exe pause
.\vtd.exe resume
.\vtd.exe stop
.\vtd.exe autostart on
.\vtd.exe autostart off
.\vtd.exe devices
.\vtd.exe transcribe recording.wav
```

`vtd copy` deliberately puts the last transcript on the clipboard. `transcribe` accepts an optional repeat count from 1 to 20 after the WAV path. To explicitly capture the next completed recording, exit the running tray and use `vtd run --capture-next test.wav`; an existing file is never overwritten.

## Update or remove

Run a newer installer against the same folder to update. It preserves preferences and reuses existing models. Installers and runtime ZIPs must come from the same release because the installer pins the payload checksum.

Uninstall through **Windows Settings → Apps → Installed apps**. Models and preferences are kept by default. Optional data cleanup removes only installer-managed settings and model filenames; external models and unrelated files are kept.

For a portable installation, run `powershell -ExecutionPolicy Bypass -File .\Uninstall.ps1` to stop that installation and remove autostart. The script keeps the files; remove the extracted folder when you no longer need it.

## Build from source

Requires Rust stable, Visual Studio C++ Build Tools and Windows SDK, CMake/Ninja, Vulkan build tools, libclang, Python and 7-Zip. The setup script supplies pinned Vulkan tools and libclang.

```powershell
.\scripts\setup-windows.ps1
.\scripts\build-windows.ps1
.\scripts\build-windows.ps1 -Test
.\scripts\package-windows.ps1
.\scripts\build-installer.ps1
.\scripts\test-installer.ps1
```

Builds use **`C:\vtd-build`** and **`C:\vtd-transcribe-build`** to avoid Windows path-length limits. Override them with `-BuildDir` and `-NativeBuildDir`. Build the `engine` and `transcribe-engine` features separately: they contain different GGML versions and must not share a process.

Whisper uses the patched whisper.cpp worker; Canary and Redux use a pinned, patched transcribe.cpp worker. Optimizations include bounded transfer buffers, compact tokenizer and graph metadata, losslessly compressed Vulkan shaders, and shorter-lived inference buffers. Model weights are not requantized by these runtime optimizations. [Native implementation and verification](docs/model-engine.md).

The default ZIP excludes models, personal preferences and development tools. `package-windows.ps1 -WithModel` includes the preset selected by the package configuration after it is downloaded. Use a separate `-OutputDir` if your normal package folder contains a running VTD.

GitHub Actions builds on manual dispatch or version tags; ordinary branch pushes and pull requests do not trigger builds. Tests and Clippy run locally to save Actions minutes. Release publication verifies the matching installer, runtime, portable ZIP and checksums before making the release public. [Installer and release instructions](installer/README.md).

## Platforms and credits

This fork targets **Windows x64 with Vulkan**, tested on Radeon RX 7800 XT. Other Vulkan GPUs and AMD Strix Halo still need hardware-specific validation; shared-memory GPUs will have different memory usage. There is no macOS build yet. The original Linux source is retained; use upstream for Linux instructions.

Based on [MQ37/vtd](https://github.com/MQ37/vtd), including the original mascot. Speech engines: [whisper.cpp](https://github.com/ggml-org/whisper.cpp) and [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp). Canary is an NVIDIA model converted by handy-computer. Redux is Moondream's ternary derivative of NVIDIA Parakeet, converted to GGUF by Nairod785.

VTD is released under the [Unlicense](LICENSE). Model and bundled-component licenses are listed in [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt).
