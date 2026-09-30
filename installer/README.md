# Windows installer

`VTD-Setup.exe` is a small native NSIS bootstrapper. It installs per user into
`%LOCALAPPDATA%\Programs\VTD` by default. The UI uses English ASCII text; speech
language defaults to the Windows display language on first installation.

The pages expose all preferences from VTD Settings: three distinct F1-F24 keys,
speech language, playback muting and startup at sign-in. The model page offers:

- The current default, Whisper large-v3-turbo Q5 (574,041,195 bytes), with a pinned SHA-256.
- A custom HTTPS URL and optional SHA-256.
- An existing whisper.cpp GGML `.bin` file, referenced without copying it.

The GGML header check catches HTML/error pages and incompatible formats; it does
not prove that an arbitrary custom model is complete or can run on the user's GPU.
Supply a checksum when distributing a custom model. HTTPS downloads use Windows
WinINet with normal certificate validation, progress, cancellation and retry/resume
within the running installer.

## Small payload

The runtime archive contains exactly two EXEs, the three imported MSVC runtime
DLLs, and license notices. It excludes models, personal configuration, build tools,
PowerShell scripts and installer plugins. It uses maximum Deflate when 7-Zip is
available, and remains compatible with Windows' built-in `tar.exe`.

The bootstrapper uses solid LZMA compression and Windows APIs for hashing, text
conversion and HTTPS. No browser, .NET runtime, PowerShell execution-policy change
or administrator permission is required. NSIS plugins exist only in its temporary
directory while setup runs. The size report lists every shipped runtime file;
build and release checks cap the bootstrapper at 300 KiB.

Windows 10/11 x64 with `tar.exe` is required (current Windows updates include it).
VTD itself requires a GPU with a working Vulkan driver.

## Build and verify

```powershell
.\scripts\build-windows.ps1
.\scripts\build-installer.ps1
.\scripts\test-installer.ps1
# Optional: real HTTPS model download, about 574 MB
.\scripts\test-installer.ps1 -RealModelDownload
```

`build-installer.ps1` downloads portable, pinned and SHA-256-checked NSIS 3.13,
INetC and nsJSON tools if needed. `-BuildDir` selects the compiled app directory;
`-OutputDir` selects staging. `-ReleaseTag` defaults to the Cargo package version
with a `v` prefix. `-Repository owner/repo` sets the GitHub download location.
`-Compression zlib` can be used for size comparisons; LZMA is the default.

Output in `dist/installer`:

| File | Purpose |
| --- | --- |
| `VTD-Setup.exe` | User-facing online installer, about 120 KB. |
| `vtd-runtime-x64.zip` | Minimal application payload, about 7 MB. |
| `SHA256SUMS.txt` | Checksums for the installer and payload. |
| `size-report.json` | Version, sizes, payload URL/hash and complete file manifest. |

Keep the matching runtime ZIP beside the installer to test before publishing or
to install offline with an existing model. A mismatched adjacent ZIP is rejected.
Without the ZIP, setup downloads the exact tagged release URL compiled into it;
that URL works only after the release is published. A test build can instead use
loopback HTTP and isolated test registry/Start menu entries. Those overrides are
never enabled in production builds.

Tests execute the real compiled installer/uninstaller: downloads, hash failures,
invalid model responses, cached model reuse, preferences, autostart, UTF-8 paths
and config preservation, rollback with a locked DLL, and data retention on
uninstall. Test fixtures do not exercise speech inference. Logs and results are
kept under `artifacts/installer-test-*`. Run these tests locally; GitHub Actions
only builds and packages releases.

## Updates and uninstall

Run the newer installer against the same folder. It reads existing preferences,
preserves other config fields and validates the resulting config through VTD.
It reuses an existing model; requesting the default again checks its SHA-256 and
avoids downloading a matching file. Downloads and validation complete before
replacing the app. Existing program files are backed up before replacement and
restored on file-copy failure. Setup closes only VTD running from its own target
folder. A setup/uninstall mutex prevents overlapping operations in one session.

The uninstaller removes app files, its Start menu shortcut and its own autostart
and Installed apps registrations. By default it keeps `vtd.json` and models.
Optional data removal deletes `vtd.json`,
`models/ggml-large-v3-turbo-q5_0.bin` and `models/custom-model.bin` only.
External model files and unrelated contents are never recursively removed.

## Unattended installation

```powershell
VTD-Setup.exe /S /OPTIONS="C:\path\options.ini" /D=C:\path\VTD
Uninstall.exe /S
# Also remove installer-owned models and config:
Uninstall.exe /S /PURGE
```

`/D=` must be the last argument and its value is not quoted, even with spaces.
Omit `/D=` for the default or previously installed location. Silent installation
does not launch VTD unless `Launch=1` is explicitly provided.

Example `options.ini` (UTF-16 is supported for non-ASCII paths):

```ini
[Settings]
HoldKey=119
ToggleKey=120
ReplayKey=121
Language=cs
Mute=0
Autostart=1
Launch=0
ModelSource=default
```

Keys are Windows virtual-key codes: F1=112 through F24=135. Booleans are 0/1.
Omitted fields preserve upgrade preferences or use first-install defaults.
For a custom download, set `ModelSource=url`, `ModelUrl=https://...` and optionally
`ModelHash=<64 hex characters>`. For an existing file, set `ModelSource=file` and
`ModelFile=C:\path\model.bin`. Exit code 0 means success, nonzero means failure.

## Releases

The Build Windows workflow runs only when manually dispatched. Ordinary branch
pushes and pull requests do not consume build minutes. Release publication happens
on a `vMAJOR.MINOR.PATCH` tag
(optionally with a prerelease suffix), or manual dispatch naming an existing tag.
The numeric version must match `Cargo.toml` at that tag.

1. Commit the tested source, workflow and version change; merge to the default branch.
2. Create and push the new version tag, for example `v0.3.0`.
3. The pipeline checks the version, builds Windows and creates the installer,
   runtime ZIP and portable ZIP. It runs no tests or Clippy. Build tools and
   compiled dependencies are cached, with a 30-minute limit for the Windows job.
4. The publish job creates a draft release, uploads all assets and verifies their
   sizes and SHA-256 digests through the GitHub API before making it public.

Published releases are never overwritten: the installer pins the payload hash.
Use a new version tag for changes. An interrupted draft can be retried through
manual workflow dispatch. Only the publish job gets `contents: write`; build and
package jobs have read access. No custom secret is required.

If publishing fails after the package artifact was uploaded, dispatch Release
Windows with the same tag and its previous `artifact_run_id`. This skips the
Windows build and reuses `vtd-windows-release` from that run. The workflow checks
that the run's source commit is exactly the tag commit before allowing reuse.

Signing is intentionally deferred. When certificates are introduced, sign the app
EXEs **before** creating either ZIP, then sign the installer **before** generating
the final `SHA256SUMS.txt`. The pipeline already marks these integration points.

Tool references: [NSIS](https://nsis.sourceforge.io/Docs/),
[INetC](https://nsis.sourceforge.io/Inetc_plug-in),
[nsJSON](https://nsis.sourceforge.io/NsJSON_plug-in).
