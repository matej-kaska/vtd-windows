# Windows speech model integration

The model catalog in `assets/models.json` drives both native Settings and NSIS
setup. It contains model names, pinned downloads, SHA-256, language support,
download sizes and benchmark labels. Existing configurations retain their model;
a new installation recommends Canary for its supported Windows language, with
Whisper as the fallback. Canary requires an explicit language.

## Runtime and optimizations

Whisper remains in `vtd-engine.exe`. Canary, Redux and legacy Parakeet use `vtd-transcribe.exe`,
linked statically against transcribe.cpp commit
`e85b30edac87533168863283c1e595bf39bd7d15`. The two GGML versions never share a
process. All engines select Vulkan explicitly, preferring a discrete GPU;
failure does not silently switch inference to the CPU.

The Windows patch strips shader debug instructions, compresses embedded shader
data with XPRESS_HUFF, decompresses before the existing driver-specific SPIR-V
fixups, releases shader modules once pipelines are created, and uses sleeping
fence waits. Only Canary and Parakeet model families are registered in this
worker. No model weights or decoding parameters are changed by these patches.

The RAM passes bound model upload and decoder readback staging to 1 MiB and
consume each FFT frame immediately in the non-BLAS frontend. The fp64 frontend
reads padding and pre-emphasis directly from the original PCM, avoiding another
whole-recording allocation. Per-feature normalization reuses its output storage.
Checkpoint filterbanks are stored once, and CPU mel, position and encoder
buffers are released after their final use. The scheduler allocates tensor
metadata in small stable arenas and sizes backend-ID arrays to the actual graph,
instead of reserving for the maximum possible number of splits.
Offline relative-position tables are generated and uploaded in tiles of at most
256 KiB for the current models. Synchronous one-dimensional Vulkan tensor
transfers are also split at 1 MiB so a later encoder upload/readback cannot
silently grow that staging allocation again.
For Canary's single-utterance path, mel and position inputs are explicitly
placed on the primary backend. The encoder result stays in an independent
backend buffer through cross-attention KV construction, avoiding a download to
RAM followed by another upload. That buffer is freed before prompt decoding.

Parakeet's embedding retains the original GGUF encoding and expands only the
requested row. In the pinned Windows AVX2 build, four LSTM matrices and both
single-vector joint projection matrices also retain original Q8_0. `native/exact_matvec.cpp` expands
blocks in SIMD registers and preserves GGML's fp32 FMA accumulation and reduction
order. Activations remain fp32; no weights are requantized. Other weight types
and unsupported build configurations use the existing fp32 implementation.
The custom operator uses the decoder's existing CPU threadpool; the encoder
continues on Vulkan. Its reference test must pass when updating GGML or compiler
flags, because a different fp32 reduction tree would invalidate bit parity.

The joint weights load directly into their final CPU storage. The encoder
projection borrows its input and keeps one result buffer through decoding,
adding the bias in place. Token lookup uses a compact table of integer indices
into the existing vocabulary instead of another copy of every token string.
Canary releases its per-recording KV cache after offline transcription; model
weights and Vulkan pipelines remain loaded until the normal idle timeout.

Long recordings split at a quiet interval between 20 and 30 seconds, retaining
every sample and a final segment of at least three seconds. This bounds encoder
workspace and avoids the decoder output ceiling. It can affect words crossing a
forced boundary. A reported truncated result is rejected instead of inserted.
Normal idle unloading still releases the entire worker after 30 seconds.

Settings downloads run in a separate thread using Windows curl over HTTPS.
The previous configuration stays active until file length and SHA-256 match.
Cancellation terminates curl and removes only its partial download. Verified
cached models are reused; custom model paths are preserved.
Each preset has its own availability label and Download/Delete action. Download
alone never commits preferences. Apply switches models without closing Settings;
Save switches and closes. Stop download cancels only the pending job. Deletion
requires confirmation and removes only a catalog file inside this installation's
models directory. The model in use, redirected directories and external files
are protected. Applied changes and completed file operations survive Cancel.

## Accuracy and quantization study

Local evidence is in `artifacts/asr-comparison`: dataset manifest, model hashes,
per-clip hypotheses, timing rounds, scripts, provenance and `summary.json`.
These large research artifacts are intentionally excluded from Git.

- Windows, AMD Radeon RX 7800 XT, Ryzen 7 7800X3D, 32 GB RAM, four worker threads.
- 100 deterministic Czech FLEURS test clips: 1,226.7 seconds, 1,964 reference words.
- Dataset revision: `70bb2e84b976b7e960aa89f1c648e09c59f894dd`.
- Unicode NFC, lowercase, punctuation ignored, accents and digits retained;
  aggregate word-level Levenshtein edits divided by reference word count.
- Native runtime defaults for the new models; existing Whisper beam size 5.
- Warm timing: ten measured runs across two rounds in reversed model order.
  The 30-second fixture concatenates and crops the first dataset clips.
- RAM and per-process dedicated GPU memory are sampled, not allocation traces.
  No power/energy consumption was measured.

| Model | Czech WER | Warm 30 s median | Model MB | Peak VRAM MiB |
| --- | ---: | ---: | ---: | ---: |
| Whisper turbo Q5_0 | 12.22% | 0.665 s | 574 | 948 |
| Canary Q8_0 | 9.73% | 0.344 s | 1,144 | 1,725 |
| Canary Q5_K_M | 9.98% | 0.341 s | 837 | 1,431 |
| Canary Q4_K_M | 9.78% | 0.330 s | 735 | 1,334 |
| Parakeet Q8_0 | 13.29% | 0.240 s | 740 | 902 |
| Parakeet Q5_K_M | 12.98% | 0.196 s | 549 | 719 |
| Parakeet Q4_K_M | 12.73% | 0.193 s | 485 | 659 |

This is a small read-speech sample, not a general accuracy guarantee or a claim
that quantization improves recognition. It supports Q4_K_M as the practical
preset for both new models. The UI explicitly identifies Czech WER and the test
hardware instead of presenting a universal accuracy percentage.

## Integrated worker verification, 30 September 2026

The optimized VTD worker reproduced **all 100 Canary and all 100 Parakeet
hypotheses exactly**, including punctuation, compared with the original native
benchmark worker. WER therefore remained 9.78% and 12.73%. Short transcripts also
matched before and after a 300-second workload; silence and tiny input returned
empty strings. The long fixture repeats a 14.1-second recording and is a
throughput/lifecycle test, not a conversational accuracy benchmark.

The following comparison uses repeated starts after GPU driver caches were
populated, the same Q4 weights, and medians of five 30-second / three 300-second
runs. Background system load remained variable; these are local measurements,
not a guarantee of the percentage improvement on another run.

| Model and audio | Original wall time | Optimized wall time | Original CPU time | Optimized CPU time |
| --- | ---: | ---: | ---: | ---: |
| Canary, 30 s | 0.355 s | 0.343 s | 0.188 CPU-s | 0.188 CPU-s |
| Canary, 300 s | 4.082 s | 3.880 s | 2.156 CPU-s | 1.734 CPU-s |
| Parakeet, 30 s | 0.209 s | 0.199 s | 0.297 CPU-s | 0.219 CPU-s |
| Parakeet, 300 s | 2.842 s | 2.239 s | 4.469 CPU-s | 3.469 CPU-s |

CPU time sums work across threads; it is not wall time or a whole-PC CPU
percentage. Both engines accumulated **0.000 CPU seconds during each sampled
three-second idle interval**, in both audit rounds. The new executable shrank
from **38,968,320 to 14,758,400 bytes (62.1%)**.

| Optimized worker | 30 s peak resident RAM | 300 s peak resident RAM | Peak dedicated VRAM across verification |
| --- | ---: | ---: | ---: |
| Canary Q4 | 108-160 MiB | 127-179 MiB | 1,334 MiB |
| Parakeet Q4 | 158-186 MiB | 175-203 MiB | 677 MiB |

The RAM ranges cover both measured starts. Driver shader compilation/cache state
affects the working set; the patch does **not** establish a meaningful reduction
in resident RAM or VRAM. Private committed memory is a different quantity and
was much higher: about 1.81-1.88 GiB for Canary and 1.03-1.13 GiB for Parakeet at
30 seconds. Do not interpret commit as physically resident RAM.

After caches were populated, model load plus the one-second warmup took about
0.77 s for Canary and 0.55 s for Parakeet. The first optimized starts took 5.08 s
and 0.98 s. No Windows/GPU caches were flushed, so these are not controlled cold
boot comparisons. Native worker prewarming also slightly changes the peak VRAM
relative to the earlier model-only study; the UI uses the integrated figures.

Evidence: `artifacts/model-engine/{baseline,optimized}/audit.json`, preserved
first-round `audit-first.json`, and `artifacts/asr-comparison/*-integrated.json`.
The Windows settings test clicks the actual dialog's model radio controls,
asserts that exactly one is checked, and verifies the corresponding download
size. This covers the grouping regression caused by static detail labels.
The resource keeps model radios contiguous and ends their group before the
action buttons and descriptive labels. Native group-navigation checks cover
both directions and wrapping, so arrow keys stay among the model choices.
Model-management tests also exercise the actual dialog with temporary files:
standalone cached download and cancellation preserve preferences; Apply verifies
the cache, notifies a test parent, keeps the dialog open, and enables deletion of
the previous model. Deletion tests protect the active file and same-named external
files while checking that the selected unused preset is actually removed.

## RAM optimization verification, 1 October 2026

This compares the preceding optimized worker with the additional RAM changes,
using the same hardware, weights, Vulkan device and four-thread configuration.
The table is the final adjacent baseline/candidate pair, after driver caches
were populated: five measured runs for 30-second audio and three for 300-second
audio, plus one warmup for each workload. Peak RAM is the full resident working
set, including shared pages, sampled every 10 ms; it is not private commit.

| Model / audio | Before peak RAM | After peak RAM | RAM saved | Before median | After median |
| --- | ---: | ---: | ---: | ---: | ---: |
| Canary Q4, 30 s | 108.5 MiB | 103.1 MiB | 5.4 MiB | 0.341 s | 0.300 s |
| Canary Q4, 300 s | 126.4 MiB | 123.0 MiB | 3.4 MiB | 3.981 s | 3.481 s |
| Parakeet Q4, 30 s | 158.7 MiB | 136.7 MiB | 22.0 MiB | 0.211 s | 0.160 s |
| Parakeet Q4, 300 s | 173.7 MiB | 153.3 MiB | 20.4 MiB | 2.117 s | 2.027 s |

Parakeet's v3 embedding alone removes exactly **15,402,840 bytes (14.69 MiB)**
of persistent storage: 20,974,080 bytes in fp32 become 5,571,240 bytes in the
checkpoint's original Q8_0 representation. Model files remain identical.
All 8,193 rows expand to bit-identical floats. The native real-model test also
checks all six LSTM tensors and the large joint output matrix against a full
reference expansion, covering multi-chunk GPU readback.

The final worker reproduced **100/100 Canary and 100/100 Parakeet transcripts
exactly**, including punctuation, against the preceding integrated worker.
Czech WER remains 9.78% / 12.73%. A separate frontend comparison checked 144
combinations of FFT size, padding, normalization, pre-emphasis, checkpoint
filterbanks and recording length; every float matched the original bit for bit.
Short audio after a five-minute recording, silence and tiny input also matched.

System CPU load stayed variable (approximately 58-71% median including the
measurement workload). The final pair kept at least 7.8 GiB available RAM. An
earlier repeat stopped when the existing memory guard saw less than 1 GiB
available; that incomplete run is excluded. Earlier starts had substantially
higher working sets, demonstrating why cache state must be held comparable.
The timings are local observations, not fixed speed guarantees. Total CPU time
on the long fixture rose from 1.469 to 1.703 CPU-s for Canary and 3.000 to 3.125
CPU-s for Parakeet; this pass does not establish lower CPU work or energy use.

Dedicated VRAM was unchanged in the paired workloads. Private committed memory
did not consistently decrease: at 30 seconds it was 1,851 / 1,845 MiB for Canary
and 968 / 1,138 MiB for Parakeet (before / after). Across starts the baseline
Parakeet peak ranged from 968 to 1,182 MiB. Do not describe the resident RAM
savings as reductions in every memory metric.

Evidence: `artifacts/ram-20261001/comparison.json`, `baseline/audit.json`,
`candidate/audit.json`, the preserved `audit-first.json` files,
`accuracy-verification.json`, `build3.log`, `checks2.log`, and
`artifacts/asr-comparison/*-ram.json`. The final worker SHA-256 is
`13f5998f3335b23b2fea230fabbcb757ebfca2f2cd1fefdc578dc6891c2b6faf`.

## Additional RAM pass, 1 October 2026

This pass compares against the immediately preceding worker
(`13f5998f...`), so the savings below do not count the embedding optimization
again. Two adjacent baseline/candidate pairs were measured in opposite orders
with populated driver caches. Each pair includes five measured 30-second runs,
three 300-second runs, and an extra warmup per workload. Ranges below contain
the two run medians or sampled peak working sets, not confidence intervals.

| Model / audio | Before peak resident RAM | After peak resident RAM | Before median time | After median time |
| --- | ---: | ---: | ---: | ---: |
| Canary Q4, 30 s | 101.7-104.5 MiB | 101.5-102.2 MiB | 0.290-0.293 s | 0.277-0.298 s |
| Canary Q4, 300 s | 122.0-122.5 MiB | 122.0-122.8 MiB | 3.338-3.371 s | 3.334-3.502 s |
| Parakeet Q4, 30 s | 136.7-136.8 MiB | 103.9-104.1 MiB | 0.165-0.189 s | 0.165-0.172 s |
| Parakeet Q4, 300 s | 153.3-154.1 MiB | 120.3-120.5 MiB | 1.970-2.043 s | 1.891-2.324 s |

The stable resident saving is about **33 MiB for Parakeet**. Its four LSTM
matrices use 6,963,200 instead of 26,214,400 bytes, and its joint output matrix
uses 5,574,640 instead of 20,986,880 bytes: exactly **34,663,440 bytes
(33.06 MiB)** less persistent weight storage. These are the checkpoint's
original Q8_0 blocks; the enclosing Q4_K_M model file remains unchanged.
The fp64 frontend also eliminates a padded PCM copy of roughly 3.66 MiB per
30-second chunk. These allocation savings occur at different lifetimes and
must not simply be added to the measured process peak.

Canary's resident working set is effectively unchanged. The scheduler change
instead reduces its **peak private committed memory**: at 30 seconds,
1,832-1,845 MiB became 1,501-1,546 MiB, saving **287-344 MiB** in the paired
runs. At 300 seconds it saved 329-333 MiB. This is Windows memory commitment,
not a claim of that much physically resident RAM being released. Parakeet's
commit was less repeatable: 30-second savings ranged from 8 to 390 MiB as
driver allocations varied. Dedicated VRAM was identical for each paired
workload. Neither model accumulated CPU time in its three-second idle samples.

System CPU medians were approximately 61-76% including the measurement workload;
at least 5.07 GiB of RAM stayed available. The long Parakeet case was slower in
one pair and faster in the other. These runs establish the resident memory
reduction, but not a reliable speedup or reduction in total CPU work. Earlier
starts also had higher working sets, so they are preserved separately and are
not used to inflate the savings. The 300-second fixture remains a repeated
short recording for throughput and lifecycle checks.

Validation of the final executable:

- All **100 Canary and 100 Parakeet Czech transcripts** match the preceding
  worker exactly, including punctuation; WER remains 9.78% and 12.73%.
- **432 frontend cases** match bit for bit, including input/output aliasing,
  very short input, padding boundaries, normalization modes and checkpoint data.
- The compact matvec matches GGML's existing fp32 operation in **960 matrix /
  input / thread combinations**, including all five real model matrices.
- The scheduler passes **72 graph computations** covering CPU, mixed Vulkan/CPU,
  graph growth/shrinkage, multiple metadata arenas and parallel input copies.
  Initial scheduler private commitment stays below the test's 16 MiB bound.
- Native CLI, frontend, synthetic float32 model, real-model weight parity,
  Rust worker tests and worker Clippy checks pass. Silence, tiny input and
  short audio after the long recording match; workers exit cleanly.

Evidence: `artifacts/ram2-20261001/comparison.json`, `comparison-pair1.json`,
`{baseline,candidate}/audit.json`, preserved `audit-pair1.json`,
`accuracy-verification.json`, `build6.log`, `checks.log`, and
`artifacts/asr-comparison/*-ram2.json`. Intermediate scheduler-only and frontend
builds are retained in the same artifact folder. The final worker SHA-256 is
`b723a00fee66e1df8565a51ed24c6a4a9c201671fe83966644003495b2db2af6`.

## Further memory audit, 1 October 2026

This pass compares the previous `b723a00f...` worker with `34659432...`.
The four changes below are additional to the earlier RAM reductions:

| Area | Change | Allocation saving |
| --- | --- | ---: |
| Tokenizer, both models | One vocabulary plus a compact index; no duplicate token strings or hash nodes | 1,313,752 bytes for Canary; 852,688 bytes for Parakeet |
| Parakeet joint prediction | Keep the sixth eligible matrix in original Q8_0, with the existing exact fp32 matvec | 1,203,200 bytes permanently |
| Parakeet projection and loading | Borrow projection input, add bias in place, retain one output; load joint weights directly into final storage | 3,465,216 bytes at projection for 376 frames; 4,297,752 bytes of load-time mirrors removed |
| Canary offline decoder | Free the completed recording's KV cache immediately | 37.75 MiB dedicated VRAM after the final 14.1-second fixture |

Tokenizer figures are differences in live Windows heap allocations measured
with the real GGUF vocabularies, not process working sets. Projection, load-time
and persistent allocations have different lifetimes. They must not be added
together and advertised as a process peak reduction. The KV cache size depends
on recording length; its allocation logs ranged from about 32 to 44 MiB in these
workloads. Releasing it does not reduce the peak while it is needed for decoding.

Two adjacent comparisons ran in opposite orders after populating driver caches.
Each worker used the same models, Vulkan device and four threads, with five
measured 30-second runs and three 300-second runs, plus workload warmups.
The table contains the range of the two sampled peaks or run medians.

| Model / audio | Before peak resident RAM | After peak resident RAM | Before median time | After median time |
| --- | ---: | ---: | ---: | ---: |
| Canary Q4, 30 s | 101.7-104.2 MiB | 102.4-102.5 MiB | 0.287-0.312 s | 0.301-0.310 s |
| Canary Q4, 300 s | 121.7-122.9 MiB | 120.5-120.6 MiB | 3.297-3.409 s | 3.345-3.399 s |
| Parakeet Q4, 30 s | 103.8-104.2 MiB | 99.6 MiB | 0.161-0.163 s | 0.160-0.162 s |
| Parakeet Q4, 300 s | 120.1 MiB | 116.1-116.3 MiB | 1.959-2.073 s | 1.913-1.928 s |

Parakeet saves another **4.2-4.6 MiB resident at 30 seconds** and 3.8-4.0 MiB
at 300 seconds. Canary's 30-second working set varies more than its small
allocation saving; the five-minute cases save 1.2-2.4 MiB. Its idle dedicated
VRAM consistently drops from **920.41 to 882.66 MiB**, while the 30-second peak
stays **1,334.18 MiB**. Parakeet's paired VRAM measurements are unchanged.

Canary private commitment at 300 seconds drops from 1,605-1,606 to about
1,358 MiB, but commitment also includes driver allocation behavior and is not
resident RAM. Parakeet's commit decreases in one pair and increases in the other.
Neither model uses CPU time in the three-second idle samples. System CPU medians
range from 59% to 72%, including the workload; at least 3.97 GiB RAM remains
available. Timings and total CPU work vary, so this pass does not establish a
general speedup. Earlier starts with much higher working sets are preserved
separately and excluded from the saving claims.

Validation of the final executable:

- All **100 Canary and 100 Parakeet Czech transcripts match exactly**, including
  punctuation; WER remains 9.78% and 12.73%.
- **1,024 exact matvec comparisons**, including all six real matrices, preserve
  the original fp32 result. All 12 decoder weight/bias tensors and 8,193 embedding
  rows match their reference expansions.
- **20 encoder projection comparisons** match bit for bit across dimensions,
  frame counts and thread counts, including unaligned borrowed input, which
  remains unmodified.
- Tokenizer tests cover duplicate pieces, missing pieces, empty and binary
  strings, copy/move, and reload with a larger or smaller vocabulary. Real
  vocabulary lookup hashes, tokenizer smoke and SentencePiece BPE parity pass.
- The existing **432 frontend cases**, scheduler CPU/Vulkan tests, native CLI
  build, float32 fallback model, Rust worker test and Clippy checks pass.
- Silence, tiny input and a short recording after the long workload remain
  correct; workers exit cleanly.

Evidence: `artifacts/ram3-20261001/comparison-round{1,2}.json`,
`{baseline,candidate}/audit-round{1,2}.json`, `accuracy-verification.json`,
`build3.log`, `checks.log`, `probes2.log`, the tokenizer/projection probe outputs,
and `artifacts/asr-comparison/*-ram3.json`. Final worker SHA-256:
`34659432c206765bf59b5e2fea77c74405ed86df7e1e7775ff7bbbe06b018b84`.

### Remaining opportunities and constraints

The audit followed recording ownership and IPC, GGUF loading, tokenizer storage,
frontend buffers, both encoder/decoder paths, scheduler arenas, Vulkan staging
and the idle/unload lifecycle. The following candidates were unimplemented at
the end of this pass. The first two are addressed by the bounded-transfer pass
below; the Canary handoff is addressed by the following Canary-specific pass.
The other rows remain candidates without measured savings:

| Area | Remaining allocation or opportunity | Constraint / next useful experiment |
| --- | --- | --- |
| Relative-position generation, both encoders | About 2.93 MiB temporary fp32 bank at 376 encoder frames | Generate and upload small tiles; verify identical sin/cos results and the cost of more Vulkan transfers |
| Vulkan synchronous staging | Model upload/readback can leave a 4 MiB staging allocation on the device | Trial a smaller bound together with tiled positions; inspect every transfer because another large upload can grow it again |
| Canary encoder to decoder handoff | About 1.47 MiB host copy at 376 frames, downloaded then uploaded for cross-attention | Retain an encoder GPU result across graph rebuilds; check whether removing host RAM instead increases retained VRAM or latency |
| Five-minute audio ownership | 18.31 MiB float32 PCM in the worker before chunked decoding finishes | Shared or directly backed recording storage could remove copies; merely streaming IPC chunks keeps the parent's original audio alive longer and does not prove lower combined RAM |
| Parakeet encoder projection weights | 2.5 MiB fp32 CPU matrix expanded from Q4_K | A new exact multi-vector kernel would be needed; ordinary quantized matmul changes activation precision and does not satisfy the current bit-exact requirement |
| Driver and allocator retention | Working set and commit vary even with identical model allocations | Use allocation traces to attribute the remainder; forcing a smaller working set only moves pages and is not an allocation saving |

The loader already releases full GGUF parsing metadata and keeps only needed
configuration. Shader decompression is temporary, shader modules are freed after
pipeline creation, and mel/position/encoder host buffers are released after use.
Both current presets use the 512-point fp64 frontend, so changing a separate
mixed-radix fp32 fallback would not save memory for these models. Default Canary
GPU decoding already reduces logits to an argmax on the GPU and reads back an
integer; downloading a full vocabulary is not the default path to optimize.

## Bounded transfer and position buffers, 1 October 2026

This pass compares `34659432...` with `bebb2cc3...`, without counting the preceding
optimizations again. It replaces each offline encoder's full host position bank
with tiles. At 376 encoder frames and width 1,024, **3,076,096 bytes become
262,144 bytes**, removing **2,813,952 bytes (2.684 MiB)** of temporary allocation.
The same helper serves single and batch offline paths; Parakeet's streaming
position cache is retained because that path reuses its contents.

Model file reads and decoder weight readback chunks decrease from 4 to **1 MiB**.
The Vulkan backend also bounds synchronous one-dimensional set/get operations
to 1 MiB, including offsets and tensor views, rather than allowing a later
transfer to recreate a larger staging buffer. Asynchronous and strided transfer
paths keep their existing behavior. Tensor contents, checkpoint encodings and
inference settings are unchanged.

Two adjacent pairs ran in opposite orders, with populated driver caches, five
measured 30-second runs and three 300-second runs per model, plus warmups:

| Model / audio | Before peak resident RAM | After peak resident RAM | Before median time | After median time |
| --- | ---: | ---: | ---: | ---: |
| Canary Q4, 30 s | 102.4-102.6 MiB | 101.8-102.3 MiB | 0.307 s | 0.307-0.309 s |
| Canary Q4, 300 s | 120.9-121.1 MiB | 120.8-121.6 MiB | 3.326-3.410 s | 3.360-3.468 s |
| Parakeet Q4, 30 s | 99.6-99.7 MiB | 96.5-96.6 MiB | 0.165-0.170 s | 0.158-0.169 s |
| Parakeet Q4, 300 s | 116.2-116.3 MiB | 113.1-113.3 MiB | 1.856-1.895 s | 1.860-1.979 s |

The repeatable process saving is **3.1 MiB for Parakeet at 30 seconds**, and
3.0 MiB at 300 seconds. Its idle resident RAM falls another 3.3-3.5 MiB.
Canary's full working set is effectively unchanged: the removed temporary
position allocation does not determine its overall process peak. Do not add
temporary allocation sizes to these resident savings. Dedicated VRAM, including
the previous pass's lower Canary idle cache usage, is unchanged in both pairs.
Private commitment remains variable and is not a physical RAM measurement.

Load plus warmup takes 0.596-0.681 s before and 0.527-0.552 s after for Parakeet;
Canary ranges overlap at 0.754-0.835 s before and 0.756-0.857 s after. These are
two starts, not a cold-boot benchmark. The long Canary workload is 1.0-1.7% slower
in these pairs; long Parakeet timing improves in one pair and worsens in the
other. No general speedup or reduction in CPU work is established. System CPU
medians range from 57% to 70%; at least 3.51 GiB RAM remains available. The
harness starts only above 3 GiB available and retains the 1 GiB runtime stop
guard. Neither model accumulates CPU time in the three-second idle checks.

Validation:

- **100/100 Canary and 100/100 Parakeet Czech transcripts match exactly** against
  the previous worker; WER remains 9.78% and 12.73%.
- **90 CPU/Vulkan position cases** match the former full-bank computation bit
  for bit, including tile boundaries, odd widths and different position origins.
- **12 CPU/Vulkan transfer cases** check partial writes, tensor views, unaligned
  host pointers, preserved surrounding bytes and sizes around the 1 MiB boundary.
- The native CLI builds; the 1,024 matvec comparisons, real-model weight parity,
  432 frontend comparisons, tokenizer/BPE, scheduler, float32 model, Rust worker
  and Clippy checks pass. Silence, tiny input and short-after-long checks match;
  all measured workers exit cleanly.

Evidence: `artifacts/ram4-20261001/comparison-round{1,2}.json`, corresponding
`{baseline,candidate}/audit-round{1,2}.json`, `accuracy-verification.json`,
`build1.log`, `checks.log`, and `artifacts/asr-comparison/*-ram4.json`.
The initial baseline with a higher working set is preserved as `audit-first.json`
and excluded from the saving claims. Final worker SHA-256:
`bebb2cc3d8b16efcc9d2476088f84a5d1de12ad68febd2d66c664d3fac297444`.

## Canary input placement and encoder handoff, 1 October 2026

This Canary-specific pass compares `bebb2cc3...` with `d1864275...`. It removes
two remaining host copies from the single-utterance path used by VTD:

- GGML normally puts graph inputs on the CPU before copying them to the GPU.
  Generating positions in tiles therefore still left a complete host tensor in
  the scheduler. Placing Canary's mel and position tensors directly on its
  primary backend eliminates **4,612,608 bytes (4.40 MiB)** of CPU scheduler
  buffers for 3,001 mel frames / 376 encoder frames. The structural test measures
  that allocation falling to **zero** for both encoder and cross-KV stages.
- The encoder output no longer travels through a **1,540,096-byte (1.47 MiB)**
  host vector at that length. A backend-to-backend copy survives the encoder
  graph reset and is consumed directly by the cross-KV graph. Its buffer and
  tensor metadata are released after that graph completes and the scheduler
  resets, before allocating the prompt graph. Offline teardown also releases
  them on error; session destruction uses the same cleanup.

These are allocation sizes at particular stages, not values to add to measured
working-set peaks. The batched fast path retains its existing packed host
handoff; VTD submits each recording chunk through the single-utterance path.
Primary backend selection, decoding defaults and token budgets are unchanged.

Two adjacent comparisons ran in opposite orders using the same model weights,
Vulkan0, four threads and populated driver caches. Each includes five measured
30-second runs and three 300-second runs plus warmups:

| Canary Q4 audio | Before peak resident RAM | After peak resident RAM | Before median time | After median time |
| --- | ---: | ---: | ---: | ---: |
| 30 s | 101.9-102.1 MiB | 96.6 MiB | 0.304-0.333 s | 0.303-0.310 s |
| 300 s | 120.9-121.0 MiB | 115.5-116.5 MiB | 3.646-3.723 s | 3.452-3.478 s |

This saves another **5.3-5.4 MiB resident RAM at 30 seconds** and **4.5-5.3 MiB
at 300 seconds**. The tradeoff is a small temporary GPU allocation: sampled
30-second peak dedicated VRAM is 1,334.18-1,335.68 MiB instead of 1,334.18 MiB,
and the 300-second peak is 1,309.93-1,311.43 MiB instead of 1,309.93 MiB. Idle
VRAM remains 882.66 MiB. Peak private commitment decreases 4-7 MiB at 30 seconds
but increases 34-35 MiB on the long workload; this driver-sensitive metric is
distinct from the measured physical working set.

The long fixture is faster in both pairs, while the 30-second result is faster
in one and slightly slower in the other. CPU work does not consistently fall.
Background CPU medians span 57-74% across the Canary and Parakeet controls;
at least 3.39 GiB RAM stays available. Both models use zero CPU time during the
three-second idle checks. Parakeet code is unchanged and its approximately
96.8 MiB 30-second working set is effectively unchanged. These measurements do
not establish a universal speed improvement or accuracy outside the test set.

Validation:

- **100/100 Canary and 100/100 Parakeet Czech transcripts match exactly** against
  the previous executable; WER remains 9.78% and 12.73%.
- The real Canary model test compares the former CPU-input/host-handoff path
  with direct backend inputs and the retained encoder tensor at three lengths.
  Encoder outputs and every cross-KV byte match bit for bit, including after
  the encoder graph is destroyed and its scheduler storage is reused.
- The real graph has 5,048 encoder nodes and uses 2,518,032 bytes of metadata
  within its 8 MiB context reservation. That reservation was not counted as
  fully resident RAM or blindly reduced to claim an additional saving.
- Native CLI, tokenizer/BPE, 1,024 matvec comparisons, real Parakeet weights,
  432 frontend comparisons, 90 position cases, 12 transfer cases, scheduler,
  float32 model, Rust worker and Clippy checks pass. Short-after-long, silence
  and tiny input results match; workers exit cleanly.

Evidence: `artifacts/ram5-20261001/comparison-round{1,2}.json`, corresponding
`{baseline,candidate}/audit-round{1,2}.json`, `accuracy-verification.json`,
`native-details.log`, `build1.log`, `checks.log`, and
`artifacts/asr-comparison/*-ram5.json`. Initial starts with higher working sets
are retained as `audit-first.json` and excluded from the saving claims.
Final worker SHA-256:
`d1864275124ae6ed7cadf349abb44ec1188b628de036f44e5de6c5400c4de139`.

## Packed graph allocation records, 1 October 2026

This pass compares `d1864275...` with `04fd96b8...`. GGML's graph allocator
previously stored ten source-allocation records for every node, including
unused source slots. It now stores only present sources in a contiguous array.
Each node holds an offset and a source-presence mask. Changes to that mask
invalidate cached allocation assignments before any packed record is accessed.
The buffer placement algorithm, tensor types and model arithmetic are unchanged.
This shared allocator serves both Canary and Parakeet.

A 5,049-node synthetic graph with mostly two inputs per node uses **565,456
bytes instead of 1,777,248 bytes** for these allocation records: **1,211,792
bytes / 1.16 MiB / 68% less**. This is an allocation measurement for that test
graph, not a claim that Canary's complete process shrinks by 68%.

Two subsequent paired runs used the same Q4_K_M models, Vulkan0, four threads,
five measured 30-second runs and three measured 300-second runs per worker,
with warmups and opposite executable order:

| Model / audio | Before peak resident RAM | After peak resident RAM | Paired saving |
| --- | ---: | ---: | ---: |
| Canary / 30 s | 96.72-97.13 MiB | 95.52-95.68 MiB | 1.20-1.45 MiB |
| Canary / 300 s | 115.69-115.85 MiB | 114.29-114.93 MiB | 0.91-1.40 MiB |
| Parakeet / 30 s | 96.52-96.64 MiB | 95.99-96.09 MiB | 0.43-0.65 MiB |
| Parakeet / 300 s | 113.41-113.59 MiB | 112.63-112.66 MiB | 0.75-0.95 MiB |

Sampled peak dedicated VRAM matches within each pair: Canary 1,334.18 MiB at
30 seconds and 1,309.93 or 1,310.68 MiB at 300 seconds; Parakeet 665.28 and
658.03 MiB. No new GPU allocation is introduced. Idle VRAM is unchanged and
idle CPU time remains zero over the three-second checks. Idle resident RAM
does not consistently decrease. Private commit remains driver-sensitive:
Canary's long workload increases 39.45 MiB in one pair and decreases 1.79 MiB
in the other; this pass does not claim a consistent commit saving.

The ordinary 30-second Canary timing pairs were about 20 ms slower with the
candidate (0.306/0.315 s versus 0.286/0.295 s). An additional check kept both
owned workers loaded and alternated AB/BA order for 16 pairs, with only one
worker transcribing at a time. Its medians were **0.319 s before and 0.324 s
after**, with every transcript identical. No speed or CPU-work improvement is
claimed. Background CPU medians in the two memory comparisons span 62.5-95.2%;
minimum available memory is 1.82 GiB. Timing is therefore load-dependent.

The initial exploratory pair is retained as `comparison-round1.json`. Both
executables had substantially higher resident working sets in that run, so its
absolute values are not combined with the subsequent pairs. One accuracy run
stopped when available memory crossed its 2 GiB guard; the interrupted evidence
is retained. A fresh run completed all 200 samples with the same guard enabled.
Memory lifecycle audits retain their 3 GiB start / 1 GiB runtime guard.

Validation:

- **100/100 Canary and 100/100 Parakeet Czech transcripts are identical** to the
  previous worker. WER remains 9.78% and 12.73% on the same reference dataset.
- `vtd_allocator_memory` compiles the actual allocator implementation and
  checks 72 CPU/Vulkan graphs: sparse inputs, the last source slot, all ten
  sources, views, external tensors, growth/shrinkage and cached source-mask
  changes. Computation is checked again after changing a cached mask.
- The Canary encoder/cross-KV bit checks, real Parakeet weights, 1,024 matvec
  comparisons, scheduler, positions/transfers, tokenizer/BPE, mel unit,
  synthetic float32 model, Rust worker and Clippy checks pass. Lifecycle inputs
  (14 s, 30 s, 300 s, silence, tiny, short-after-long) match and workers exit
  cleanly. The preceding 432 frontend bit comparisons were not rerun because
  the frontend implementation did not change.

The packed records still retain sentinels for present sources that already own
storage or are views. Eliminating those records would require preserving the
same invalidation and view-initialization rules; no additional saving is claimed.

Evidence: `artifacts/ram6-20261001/comparison-round{2,3}.json`, corresponding
`{baseline,candidate}/audit-round{2,3}.json`, `accuracy-verification.json`,
`native-details.log`, `allocator-final.log`, `build1.log`, `checks.log`,
`timing.json`, and `artifacts/asr-comparison/*-ram6.json`.
Final worker SHA-256:
`04fd96b8cffaef60ed8d88a2f715345e9a7bc0f518faf44aafc939233359b64a`.

## Rebuilding and verification

Redux replaces the Parakeet preset in the source catalog. Its `TQ1_G128` GGUF
storage type (96) is recognized by the pinned GGML loader, then retyped to
Q4_0 before tensor validation/allocation. Streaming upload repacks each group
losslessly with combined input/output staging at most 1 MiB. Dense tensors
follow the existing upload path. Quantized pointwise kernels use validated
`[in, out]` matrices and direct matmul; legacy dense convolution shapes and
behavior are retained. All application inference still requires Vulkan.

The conversion is tested by `vtd_redux_weights`; the real-model smoke test
also accepts the Redux variant and checks the v3 vocabulary/architecture.
See [Redux integration evidence](redux-benchmark.md#vtd-integration-4-october-2026)
for accuracy, memory, long-audio checks and the distinction between compact
storage and the expanded fast GPU runtime.

Run `scripts/build-windows.ps1`; it obtains the pinned native checkout, applies
`patches/windows/transcribe.patch` idempotently, and builds each engine separately.
The CMake hook detects the MSVC include prefix in UTF-8 so localized compiler
output cannot silently hide changed header dependencies from Ninja.
Use `-Test` for local Rust tests. `scripts/test-installer.ps1` tests the real setup
and uninstall executables using an isolated registry key and loopback fixtures;
when local real models are available it also verifies preset reuse and upgrades.
For the compact matvec oracle, enable native `TRANSCRIBE_BUILD_TESTS` and
`TRANSCRIBE_BUILD_REAL_MODEL_TESTS`, set `TRANSCRIBE_PARAKEET_GGUF` to the local
Parakeet v3 Q4_K_M model, and run CTest `vtd_exact_matvec`. Without the fixture
environment variable it reports a skip. `vtd_scheduler_memory` needs Vulkan0
and exercises both CPU-only and mixed CPU/GPU execution.
CTest `vtd_positions` also needs Vulkan0 and checks the tiled position values
and bounded transfers against CPU references and byte patterns.
CTest `vtd_canary_memory` requires `TRANSCRIBE_CANARY_1B_V2_GGUF` and Vulkan;
it checks real encoder/cross-KV bit parity and CPU buffer allocation sizes.
CTest `vtd_allocator_memory` needs Vulkan0 but no model fixture; it verifies
packed allocation records, source-mask changes and exact CPU/Vulkan results.

`settings-preview` is an optional development feature. With it,
`vtd __settings-preview` opens a read-only settings window for visual checks
without attaching to the user's running tray. It is absent from release builds.

The original models are published by
[NVIDIA Canary](https://huggingface.co/nvidia/canary-1b-v2) and
[NVIDIA Parakeet](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3), under CC BY 4.0.
The catalog uses handy-computer's GGUF conversions. Attribution and the license
link are included in `THIRD_PARTY_LICENSES.txt` shipped with both packages.
