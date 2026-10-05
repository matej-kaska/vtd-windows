# Parakeet Redux experiment — 4 October 2026

Redux was tested locally on AMD Vulkan and in a separate CPU-only executable.
The initial experiment below used a separate transcribe.cpp checkout. The
subsequent VTD integration is described at the end of this report.

## Method

- Windows, Ryzen 7 7800X3D (8 physical cores), Radeon RX 7800 XT, 32 GB RAM.
- CPU build: MSVC x64, AVX2/FMA/F16C/BMI2, no AVX-512, no Vulkan dependency.
  CPU affinity and compute threads both limited to the reported core count.
  This is a modern desktop with fewer cores enabled, not an old laptop test.
- Same 100 deterministic Czech FLEURS clips, 1,226.7 seconds, 1,964 words,
  dataset revision `70bb2e84b976b7e960aa89f1c648e09c59f894dd`.
- Same existing VTD lexical scoring: Unicode NFC, lowercase, punctuation split,
  accents and digits retained; aggregate Levenshtein word edits / reference words.
  Every source audio SHA-256 was verified against the existing manifest.
- Warm timing: one warmup, then 7 GPU / 5 CPU repeats of the same 30-second
  fixture. Load time is separate and benefits from an already cached model file.
- RAM below is the Windows process peak working set from startup through the
  final 30-second repeat, before the WER/long-audio runs. It includes shared pages;
  private commit is recorded separately in the JSON and is not physical RAM.
  VRAM is per-process dedicated GPU memory sampled every 50 ms. These are
  process metrics, not the total memory needed to run Windows and other apps.
- Other applications remained active. Pre-run CPU load and per-run time ranges
  are retained below; speed differences are provisional. No energy measurement.
- GPU comparisons use the same fork and benchmark worker for both models.
  They do not isolate Redux against the production VTD executable's RAM patches.

## AMD Vulkan

| Model / runtime | Model MB | Warm 30 s, median | RAM peak MiB | VRAM peak MiB | Czech WER |
| --- | ---: | ---: | ---: | ---: | ---: |
| Parakeet v3 Q4_K_M | 485.4 | 0.153 s | 187.9 | 658.7 | 12.73% |
| Redux, Q4 runtime | 159.1 | 0.174 s | 154.6 | 545.1 | 12.32% |
| Redux, native ternary | 159.1 | 0.192 s | 188.8 | 347.1 | 12.32% |

Both Redux Vulkan layouts produced byte-for-byte identical transcripts on all
100 clips. Redux made 242 word edits versus
250 for Parakeet v3: 8
fewer edits, 0.41 percentage points lower WER. This is
a small difference: the paired 95% bootstrap interval for Redux minus v3 is
[-2.30, 1.32] percentage points
and includes zero. Canary's earlier result on this exact corpus was 9.78%; it
remains the stronger measured accuracy option (not rerun in this experiment).

## CPU only

| Model / runtime | Cores / threads | Warm 30 s, median | RAM peak MiB | Czech WER |
| --- | ---: | ---: | ---: | ---: |
| Parakeet v3 Q4_K_M | 2 | 3.99 s | 979.3 | timing only |
| Redux, Q4 runtime | 1 | 9.51 s | 576.8 | timing only |
| Redux, Q4 runtime | 2 | 4.24 s | 577.4 | 12.42% |
| Redux, Q4 runtime | 4 | 2.23 s | 577.0 | timing only |
| Redux, native ternary | 2 | 16.03 s | 378.8 | 12.12% |

CPU inference used the strict CPU backend in a binary built without Vulkan.
No dedicated GPU allocation was observed. The two-thread Redux configurations
each completed all 100 Czech clips; one/four-thread and original-v3 CPU cases
measure latency only, so they do not establish a separate WER score.

For an older notebook, the faster Q4 runtime is the first candidate. The native
ternary runtime trades substantially more compute time for less RAM. AVX2 and
the other listed instruction sets are required by this experimental build;
machines without them need another build and another performance test. Fewer
cores on a Ryzen do not reproduce an older CPU's clocks, caches, thermals or RAM.

## Timing spread and background load

| Experiment | Warm min–max seconds | CPU load before run |
| --- | ---: | --- |
| parakeet-vulkan-fork | 0.143–0.176 | 53.1%, 54.3%, 53.8% |
| redux-vulkan-q4 | 0.171–0.188 | 63.8%, 74.3%, 81.4% |
| redux-vulkan-native | 0.185–0.209 | 62.3%, 68.0%, 58.9% |
| parakeet-cpu-q4-t2 | 3.657–4.143 | 72.5%, 44.6%, 39.8% |
| redux-cpu-q4-t1 | 7.194–34.785 | 53.5%, 50.4%, 56.7% |
| redux-cpu-q4-t2 | 3.614–4.702 | 54.1%, 41.9%, 56.9% |
| redux-cpu-q4-t4 | 2.091–3.369 | 58.8%, 46.2%, 44.4% |
| redux-cpu-native-t2 | 15.047–18.251 | 72.5%, 50.9%, 55.7% |

## Long-audio stress check

The GPU tests also transcribed a synthetic 300-second fixture (the 30-second
audio repeated ten times). This is a single unsegmented engine call, not VTD's
production segmentation and not an accuracy benchmark.

| Redux layout | Time, single cold-length run | Sampled RAM peak MiB | Sampled VRAM peak MiB | Output words |
| --- | ---: | ---: | ---: | ---: |
| Redux, Q4 runtime | 2.543 s | 290.1 | 1695.9 | 410 |
| Redux, native ternary | 1.684 s | 305.4 | 1497.4 | 410 |

Long unsegmented audio still needs much more workspace than a short dictation.
Integration should retain VTD's bounded segmentation and existing memory work.
The model's file size must not be used as its RAM/VRAM requirement.

## Provenance and reproduction

- [Redux weights](https://huggingface.co/Nairod785/parakeet-redux-gguf):
  `parakeet-redux-0.6b-TQ1_Q8_0.gguf`, 159,121,504 bytes,
  revision `87cbc354ce32bc9fe144b5b7bcdd9c68538907a9`, SHA-256
  `74f43ba852479e86e29df92cdbc89aa8215c7e8070f711be424ff466415b6184`.
- [Engine fork](https://github.com/NairoDorian/transcribe.cpp):
  `ba949120d60f29daaaa13eec65b9c28c2c2112a6`, unmodified source.
- Parakeet v3 Q4_K_M model SHA-256:
  `b68557be1e3c40207fd7c4bd9d63f1d3316b963f15325bfb0cc16a8bb0ffd181`.
- Exact executable hashes, transcripts, timings, process memory samples and
  exit codes: `artifacts/redux-20261004/*.json`, `*.log`, `summary.json`.
- Reproduction harness and build configuration are in that same local artifact
  directory; raw artifacts and model files remain excluded from Git.
- Each benchmark worker was terminated normally after its experiment, with a
  kill/wait cleanup path on error; no model server was left running.

```powershell
& artifacts/redux-20261004/build.ps1 -CpuOnly
python artifacts/redux-20261004/benchmark.py my-redux-cpu-run --backend cpu --threads 2

# Direct CPU transcription of a 16 kHz mono WAV:
$env:TRANSCRIBE_TERNARY_RUNTIME = 'q4_0'
& artifacts/redux-20261004/build-cpu/bin/transcribe-cli.exe `
  -m artifacts/redux-20261004/parakeet-redux-0.6b-TQ1_Q8_0.gguf `
  --backend cpu --threads 2 --timestamps none -l cs recording.wav
Remove-Item Env:TRANSCRIBE_TERNARY_RUNTIME
```

This experiment does not establish a universal speed or accuracy improvement.
The CPU package remains a separate trial; VTD retains its Vulkan requirement.

## Portable CPU trial

Local archive: `artifacts/redux-20261004/redux-cpu-test.zip`
(152.2 MB, including the model).
SHA-256: `630ef5672f32b6d165a219af55c1420d5eef084979c3ae9efbef0e9d1f05d729`.

Extract the whole archive, then drag a 16 kHz mono WAV onto `Fast.cmd` or
`LowMemory.cmd`. Both use two CPU threads. The transcript and diagnostic log
are saved under `results/`. This is a command-line trial, not a VTD installer.
It needs Windows 10 version 1903 or newer / Windows 11 and the CPU features
listed above. The Microsoft C++ runtime DLLs are included.

The final ZIP was extracted into a directory with Czech characters; every
manifest file hash was verified. Both launch modes successfully processed a
WAV with Czech characters and spaces in its filename, and their UTF-8 output
matched the corresponding benchmark transcript exactly. The example CLI
needed a UTF-8 Windows application manifest to make non-ASCII model paths
work; this was supplied by the external test build without changing the fork's
source. Package evidence: `artifacts/redux-20261004/package-verification.json`.

## VTD integration, 4 October 2026

The preset named **Redux** replaces Parakeet in Settings and Setup. It uses
the same pinned 159,121,504-byte TQ1_Q8_0 file as the experiment, verified by
SHA-256. The optimized VTD transcribe.cpp engine retains its pinned revision
and existing RAM patches; only Redux storage loading and quantized pointwise
convolutions are added. Vulkan is still required. Canary remains the default
for supported languages; existing configurations keep their model path.

TQ1_G128 tensors are repacked to Q4_0 before GPU inference. This is an exact
representation change: ternary values and FP16 scales are preserved, with
combined source/output scratch bounded to 1 MiB. It selects the fast runtime,
not the experimental native-ternary memory mode. The ternary file stays small
on disk, while its GPU weight buffers are larger than the download.

Validation of the integrated worker:

- All 100 Czech transcripts exactly match the experimental Redux Q4 Vulkan
  worker: 242 edits / 1,964 words, **12.32% WER**.
- The original Parakeet and Canary control runs each preserve all 100 previous
  transcripts, with 12.73% and 9.78% WER respectively.
- Warm 30-second median: **0.207 s**, sampled peak VRAM **563.15 MiB** across
  loading, timing and the 100-clip run. The catalog rounds these to 0.21 s and
  563 MiB. Background CPU load was high; this is not evidence of a universal
  speed improvement over original Parakeet.
- Separate sequential memory/long-audio check: **96.18 MiB** peak working set
  during 30-second audio and **112.55 MiB** during the 300-second fixture.
  The latter took 2.63 s (median of three warmed repeats), using VTD's existing
  30-second segmentation. Silence returns no text; a short clip after the
  long run matches the same clip before it. The worker exits cleanly.
- RAM is not a fixed bound: the earlier accuracy run reached **154.41 MiB**
  including startup. The unchanged installed Parakeet worker also reached
  148.57 MiB in a fresh control. Driver/process state and background load affect
  these values; cross-run RAM figures should not be treated as isolated savings.
- Native conversion tests verify 1,342,720 weight codes and scale bits across
  chunk boundaries. Existing allocator, scheduler, position, Canary-memory,
  exact-matvec and model-smoke tests pass, including loading the Redux fixture.
- All 34 installer cases passed, including Redux with Czech and automatic
  detection, reuse of verified weights, upgrades and deletion. The production
  Settings helper was checked on an isolated desktop with a test tray:
  Redux's radio, Apply, active-model deletion guard, unused-model deletion and
  missing-model download controls work. The final packaged worker's diagnostic
  transcript matches the benchmark. Existing preferences and autostart survive.

Local evidence: `artifacts/redux-integration-20261004/`, and the
`*-integrated.json` / `*-redux-control.json` records in
`artifacts/asr-comparison/`. Historical tables above describe the isolated
experimental fork, not the integrated VTD worker.
