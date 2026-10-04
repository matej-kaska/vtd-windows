# Native Windows tray footprint

## Fresh restart, v0.4.2

After a clean VTD restart on 2026-10-04, a read-only 30-second idle sample ended
at **108 KiB private working set**, **264 KiB total working set** and
**1,020 KiB private committed memory**. Private working set ranged from 84 to
108 KiB during the sample. CPU-time and read/write byte deltas were zero.

This is the running Windows tray with its original icon, no model loaded, and
no settings or menu opened after restart. The configuration was unchanged.
These are measurements on one machine, not a fixed memory limit; working set
and committed memory are different metrics. Local raw evidence is stored in
`artifacts/release-0.4.2/idle-after-restart.json`.

## Earlier measurements

Measured locally on Windows on 2026-10-04. These are observations on one machine,
not fixed memory limits. The portable app uses Canary, Czech and the existing
30-second model idle timeout. The new resident was running from `dist/vtd-windows`.

| Metric | Previous VTD | Native VTD | RAMCleanup |
| --- | ---: | ---: | ---: |
| Private working set | 1,184 KiB | 112 KiB | 104 KiB |
| Total working set, including shared pages | 6,088 KiB | 268 KiB | 272 KiB |
| Private committed memory | 2,932 KiB | 1,052 KiB | 1,060 KiB |
| Resident executable | 665.5 KiB | 22 KiB | 32 KiB |
| Threads | 3 | 2 | 1 |
| Loaded DLLs | 37 | 15 | 16 |

The previous VTD sample lasted 15 seconds; the live native VTD and RAMCleanup
samples lasted 30 seconds. Their measured CPU-time and read/write byte deltas
were all zero. DLL enumeration can itself affect page residency. Memory also
varies with Windows messages, keyboard activity and system pressure.

The final integration build also measured 108 KiB private working set after
saving settings, real model transcription, custom shortcuts, clipboard copy,
repeated replay and crash recovery on an isolated window station. Its private
commit was 1,168 KiB. That sample is separate from the live desktop measurement
above; the test build contains a synthetic input entry point and runs without
Explorer. Neither recording nor settings helpers remained alive at idle.

## Further allocation reductions

A second local optimization pass on 2026-10-04 kept the icon resources and pixels
unchanged. The running production app was replaced after local verification.
The existing `vtd.json` remained byte-for-byte identical.

| Controlled isolated integration run, after all actions | First native resident | Further optimized resident |
| --- | ---: | ---: |
| Private working set | 112 KiB | 104 KiB |
| Private committed memory | 1,072 KiB | 1,000 KiB |
| Total working set, including shared pages | 272 KiB | 260 KiB |

These are separate test builds on isolated window stations using the same real
Canary model, Czech WAV, shortcuts, settings and crash-recovery actions. Both
end samples lasted 15 seconds. Windows page residency varies, so neither result
is a guaranteed memory ceiling. The committed-memory reduction was 72 KiB.

On the actual desktop, opening and closing the tray menu and settings in the
production build then gave a 30-second idle sample of 120 KiB private working
set, 276 KiB total working set and 1,020 KiB private commit. CPU time and read/write
byte deltas were zero. The icon remained registered before and after the real
UI calls. Resident-owned GDI objects decreased from 7 to 4, and USER objects from
5 to 4. The resident executable is now 22.5 KiB; the helper remains 583 KiB.

The additional changes are:

- Once on transition to idle, `HeapSetInformation(HeapOptimizeResources)` asks
  Windows to decommit unused caches in this process's heaps before the existing
  working-set trim. This can reduce committed allocations, rather than merely
  evicting their resident pages. See [Microsoft's HeapSetInformation documentation](https://learn.microsoft.com/en-us/windows/win32/api/heapapi/nf-heapapi-heapsetinformation).
- The last transcript uses a 1,024-byte inline buffer for short UTF-8 text.
  Longer text uses a page-rounded `VirtualAlloc` reservation, which is entirely
  released when replaced. A test verifies `MEM_FREE` after a long transcript is
  replaced by a short one. Cross-process readers block reentrant writes until
  their synchronous copy completes.
- Common helper command lines use a small stack buffer. Long CLI and capture
  arguments retain the original Windows limit and use a temporary allocation.
- Identical tooltip/status updates skip repeated shell loading and title work.
  Bootstrap trims once after configuration completes, avoiding immediate startup
  page faults from an earlier, premature trim.
- Icon registration loads the same pixels into a temporary owned icon. Explorer
  keeps its displayed copy; the resident releases its USER/GDI resources after
  registration instead of retaining a shared resource handle.

The Windows segment-heap manifest option was also measured. It increased this
small process's footprint, so the production manifest retains the default heap.
Diagnostic builds that skipped shell/theme calls were used only to investigate
dependencies; production keeps the full tray behavior and dedicated hook thread.

Local validation for this pass: 24 unit tests, 16 native integration cases,
31 installer cases and Clippy with warnings denied passed. A live Win32 probe
opened and closed the real menu and settings and verified tray registration.
The separate isolated stress probe delivered 3,000 ordinary key presses,
50 pause/resume cycles and three settings open/close cycles. No helpers remained
after these actions, and no tests or runs were added to GitHub Actions.

## What changed

`vtd.exe` is a Win32 `no_std` resident with a native entry point, small stacks and
no Rust/C runtime. It owns the tray, shortcut definitions, input epoch and the
last UTF-8 transcript. A dedicated native thread pumps the low-level keyboard
hook so shell calls and process creation cannot block it.

`vtd-helper.exe` owns settings, menus, CLI commands, microphone capture,
clipboard restoration and the speech worker connection. Startup uses a brief
configuration helper that exits after transferring the shortcut preferences.
Recording launches a helper on demand and queues down/up events until it is
ready. The helper exits after its model unloads and its clipboard work finishes.
Replay can start a fresh helper using the resident's last transcript, without
loading a model. The configured timeout of zero intentionally keeps the model
and recording helper warm.

The resident loads Shell32 only for individual tray operations and releases its
reference immediately. It has no polling timer or background audio thread.
Its tray icon contains the original 16-64 pixel images; the settings and
installer retain the complete icon. Release dependencies also use aborting
panics, so they do not retain unused unwinding code.

Native kill-on-close jobs release helpers and their model workers if either
owner crashes. Normal session shutdown waits for clipboard restoration. An
explicit idle handoff and key acknowledgements protect inputs arriving while
a helper starts or exits. Settings cannot open while shortcut actions are still
pending. CLI forwarding preserves arguments, standard handles and exit codes.

Working-set trimming happens once when transitioning to idle; it is never
scheduled periodically or performed for each key. Trimming only removes
resident pages. Actual allocation reductions come from the smaller resident
and from terminating the processes that own audio, UI and model allocations.

## Local verification

- 24 unit tests passed, including configuration, hotkeys, model settings,
  downloads and registry rollback. The separate isolated clipboard test exited
  successfully; the standard unit run leaves hardware-dependent tests ignored.
- 16 native integration cases passed in `scripts/test-resident.py`, using a
  real model and a known Czech WAV. They cover fast press/release, replay after
  helper exit, Unicode clipboard copy, pause, saving preferences, modifier and
  media keys, cancellation, rapid handoffs, and owner/worker crashes.
- The production helper opened the real microphone on an isolated window
  station, captured 0.92 seconds of audio and released all its processes.
  Shortcut-to-recording status took 109 ms. The first packet arrived 131.2 ms
  after entering microphone setup, which is a different timing origin.
- Canary, Parakeet and Whisper transcribed the reference WAV through the new
  CLI wrapper. Hidden-console and redirected-output checks also passed.
- 31 local installer cases passed, including download, upgrade, rollback and
  uninstall. Both portable and installer payloads contain `vtd-helper.exe`.
  Test cases run locally. GitHub Actions only builds and verifies release assets.
- Clippy completed with warnings denied for the resident, helper and library.
- Six interactive insertion cases and four partial-send cases passed locally,
  covering real text insertion, clipboard restoration and modifier releases.
  Normal F9/F10 dictation was also confirmed by the user before this release.

The GUI tests used a separate window station and clipboard. Live desktop
clicking and visual inspection were unavailable because the Computer Use native
pipe could not be connected. The live resident measurement is from the ordinary
user desktop. The existing configuration stayed byte-for-byte unchanged.

## Reproduce

### Paste regression verification

The original isolated tests did not prove insertion into a foreground text
field. An interactive regression test reproduced a helper panic at the first
Ctrl key-down: Windows delivered `WM_RENDERFORMAT` inside `SendInput`, and the
clipboard handler attempted to borrow the application's already-borrowed state.
The aborted call never emitted Ctrl key-up.

Clipboard messages now post the idle check for later, after insertion releases
that state. A partial Ctrl+V send also releases any keys pressed by its accepted
prefix before restoring the clipboard. The resident's icon and memory layout
are unchanged.

`scripts/test-insertion.py` uses actual `SendInput` events, the low-level keyboard
hook, the existing model and a Czech WAV, and an owned native Edit control on the
interactive desktop. It verifies F9 insertion, F10 after helper exit, repeated
F10, F8 release, target rejection after typing, and Ctrl+Shift+F10. It checks
the field's exact text, restored clipboard and released modifier keys. Separate
partial-send cases exercise prefixes of zero through three events. Test-only
input tagging, WAV substitution and partial-send injection are compiled out of
shipping builds.

Run the two integration scripts sequentially; they share VTD's instance lock.
The interactive script temporarily activates its own text window, refuses to
type after losing focus, and restores the previous foreground window on exit.

Build the tray/helper without rebuilding the speech engines:

```powershell
./scripts/build-tray.ps1
./scripts/build-tray.ps1 -Test
```

Run the isolated native integration tests with existing runtime/model files in
`dist/vtd-windows` and a known speech WAV:

```powershell
./scripts/build-tray.ps1 -ResidentTest
python scripts/test-resident.py --wav path/to/reference.wav
python scripts/test-insertion.py --wav path/to/reference.wav
python scripts/test-insertion.py --wav path/to/reference.wav --paste-limit 1
./scripts/build-tray.ps1
```

The final command restores shipping builds without test-only hooks or WAV
capture substitution. Hardware verification is optional, using `--microphone`
in place of `--wav`. `--helper` can select an exact production helper.

Local JSON/log evidence is under `artifacts/tray-optimization-20261004` and the
corresponding `artifacts/resident-test-*` and `artifacts/installer-test-*`
directories. Those directories are ignored and contain no published release.
Local setup and runtime ZIP must be kept together; the release URL embedded in
this development installer still points to the previously published version.

Primary API references: [low-level keyboard hook](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc),
[WM_COPYDATA](https://learn.microsoft.com/en-us/windows/win32/dataxchg/wm-copydata),
[working-set trimming](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-setprocessworkingsetsize).
