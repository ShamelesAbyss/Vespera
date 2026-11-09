# Local release validation

Vespera has an internal release candidate workflow. Christopher Dodd approved the Proprietary Freeware License v1.0 as the project's software licensing policy. Public distribution still requires independent legal review, third-party rights verification, artifact-specific notices, and Windows signing. Keep the owner-approved license and third-party notices with internal packages. [License](../LICENSE) · [Third-party notices](../THIRD_PARTY_NOTICES.md)

## Build and stage

Use the pinned Rust toolchain, Git and the platform build tools. Start from a commit with a clean tracked worktree. Run the packager from the workspace root. [Build requirements](../README.md#requirements)

```powershell
cargo run --locked -p vespera-app --example package-local -- --output target/vespera-local
```

The same Cargo command works in a POSIX shell. Add `--offline` after the final `--` to require a cached dependency set for the release build. To keep the entire invocation offline, also put `--offline` before `-p`. Add `--headless` after the final `--` to stage the CLI alone. Use a fresh output directory with an existing parent.

The tool builds the release executables with the lockfile and stages the requested binaries, tracked configuration examples, licenses, branding, screenshots and operator documentation. It copies tracked materials from the recorded Git revision. `manifest.json` lists the source revision, Cargo.lock hash and SHA-256 and byte size of each staged file. A staging error can leave a partial directory. Treat a directory without a manifest as incomplete.

On Windows, open `bin\vespera-launcher.exe` for the native desktop without a console. The graphics build requires the Windows SDK resource compiler. Home shows the official icon on its first rendered frame. Other screens use the same image in the shared header. The desktop also embeds the PNG and Windows icon resources, so it can render its branding from any working directory. The portable package includes the unchanged original and the derived PNG, multi-resolution ICO and ICNS. The ICNS is available for future macOS app packaging. Run `./scripts/generate-branding.ps1` on Windows to regenerate derivatives from the canonical JPEG.

## Verify and start

From the repository root, the Rust verifier works in PowerShell and POSIX shells:

```sh
cargo run --locked -p vespera-app --example verify-package -- --package target/vespera-local
```

It checks schema 1 manifest entries against file sizes and SHA-256 hashes and
rejects missing files, duplicate paths, path traversal and symbolic-link entries.
Use a staged or freshly extracted directory with no concurrent writers. The
manifest is not signed: this checks listed-file integrity, not authenticity,
unlisted files, distribution rights or release readiness. It never executes
packaged binaries or modifies the package.

From the workspace root in PowerShell, verify the manifest and make a private scratch area inside the staged directory.

```powershell
Set-Location target/vespera-local
$manifest = Get-Content manifest.json -Raw | ConvertFrom-Json
foreach ($entry in $manifest.files) {
    $file = Get-Item -LiteralPath $entry.path
    $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
    if ($digest -ne $entry.sha256 -or $file.Length -ne $entry.bytes) {
        throw "Candidate file mismatch in $($entry.path)"
    }
}
.\bin\vespera.exe --version
.\bin\vespera.exe validate-config --config config/local.toml --json
.\bin\vespera.exe doctor --json
New-Item -ItemType Directory -Path data | Out-Null
.\bin\vespera.exe world --database data/check.sqlite3 init --spec config/world.example.json
.\bin\vespera.exe world --database data/check.sqlite3 advance --ticks 8
.\bin\vespera.exe world --database data/check.sqlite3 replay --revision 8
.\bin\vespera-view.exe --atlas --world data/check.sqlite3
```

On POSIX shells, use `cd target/vespera-local`, `mkdir data`, and `./bin/vespera` or `./bin/vespera-view` with the same arguments. On Linux, checksum verification can use `jq -r '.files[] | "\(.sha256)  \(.path)"' manifest.json | sha256sum -c -`. The graphics command requires the package with Atlas, a working window system and a supported wgpu adapter. The headless commands require neither a model backend nor a graphics device.

For inference, copy the example configuration to a private file and follow the provider and agent instructions. Keep the staged example unchanged so its manifest continues to verify. [CLI reference](CLI.md)

## Build the Windows installer

The x64 per-user installer uses the verified Windows graphics candidate as its input. Use the WiX command-line compiler and its matching `WixToolset.BootstrapperApplications.wixext` extension. The local rehearsal used WiX 5.0.2. Run from the repository root after verifying the staged manifest.

```powershell
$stage = (Resolve-Path target/vespera-local).Path
$version = (Get-Content (Join-Path $stage 'manifest.json') -Raw | ConvertFrom-Json).vespera_version
wix build -arch x64 -d "Stage=$stage" -d "Version=$version" -o target/vespera.msi packaging/windows/vespera.wxs
$msi = (Resolve-Path target/vespera.msi).Path
wix build -arch x64 -ext WixToolset.BootstrapperApplications.wixext -d "Stage=$stage" -d "Version=$version" -d "Msi=$msi" -o target/Vespera-Setup.exe packaging/windows/setup.wxs
```

`Vespera-Setup.exe` shows the official icon and installs the desktop, launcher, CLI, and branding assets under `%LOCALAPPDATA%\Programs\Vespera`. Both the Start Menu and desktop shortcuts target the console-free launcher. The desktop's world and agent data stay in its separate per-user data directory. Signing and public distribution remain release gates.

For an authorized local install, close Vespera and back up its data before starting Setup. From the repository root, run the following commands and complete the per-user installer.

```powershell
Get-FileHash -Algorithm SHA256 target/vespera.msi,target/Vespera-Setup.exe
Start-Process -FilePath (Resolve-Path target/Vespera-Setup.exe).Path -Wait
& (Join-Path $env:LOCALAPPDATA 'Programs/Vespera/bin/vespera-launcher.exe')
```

Use a higher three-component product version for each MSI upgrade. The `0.1.2` candidate supersedes `0.1.1`. Old-product removal is part of the Windows Installer transaction. Use the same committed candidate for both the MSI and its Setup bundle.

## Upgrade and recovery

Stage the new binaries in a fresh directory. Stop Vespera writers before making operator backups. Use `memory-admin backup` for the memory store. For stores without a Vespera backup command, preserve a consistent stopped-state SQLite copy, including associated WAL and SHM files when present. Keep each database with its own backup set. Include the desktop profile's `reduced-motion.enabled` marker when present.

Rehearse the new binaries on copies of the backed-up stores. Verify the catalog, tasks, workflow results and journal cursor ranges you rely on before switching the live paths. Broker migrations run on startup with `--broker-db`. Broker schema 3 adds durable mailboxes. Event schema 8 adds optional mailbox acknowledgement events and requires updated readers when enabled. [Broker recovery and compatibility](PROTOCOL.md#durable-broker-recovery)

For rollback, use the previous binaries with their pre-upgrade backups and compatible configuration. Preserve the upgraded stores separately for reconciliation. Reopening an upgraded live database with an older binary is only supported where the subsystem explicitly documents compatibility.

A profile using Windows Credential Manager stores the source choice and endpoint. The Windows account supplies the credential independently of the profile backup. Use the pre-upgrade profile with builds that predate the `credential_windows` field.

## Release evidence

- Record the Git revision, Windows or Linux architecture, build profile and package checksums.
- Verify the fresh-directory walkthrough, headless and graphics builds, provider matrix, permission checks and backup recovery.
- Regenerate the dependency graph and declared license inventory from the locked build. Review the actual artifacts, license texts and notices before distribution. [Dependency record](DEPENDENCIES.md)
- Record remaining compatibility and release gates in the milestone ledger. [Milestone ledger](MILESTONE_LEDGER.md)

### Windows x64 headless source rehearsal

The optimized `0.1.2` CLI was built offline from
`72dbf86169de6ea4233430766592d891ce7a5531` using `package-local --headless`.
The internal `vespera-headless-72dbf86.zip` had SHA-256
`F20EDFCEC265B8F5D5EAB1D71DD65359366C3849E854EE7B61FD7F2F1A7176FB`.
A fresh extraction verified all 43 manifest entries. The packaged CLI passed
`--version`, `validate-config` and `doctor`, initialized a fresh world, and
advanced it by eight ticks. A separate CLI invocation replayed revision 8 and
returned the same world snapshot after normalizing JSON object key order. The
manifest verified again after the walkthrough. This rehearsal covers Windows
headless operation. The desktop and installer retain separate acceptance records.

### Windows x64 desktop package rehearsal

Source `472cfe4e7202ab2758e51726feacf0a21609376e` built offline as an optimized
Windows x64 desktop package at software version `0.1.2`. A fresh portable
extraction passed all 45 manifest checks. WiX 5.0.2 built the MSI and Setup.
Administrative extraction matched all 13 MSI payloads to the portable manifest.
The extracted Setup bundle contained the byte-identical standalone MSI.

| Local rehearsal artifact | SHA-256 |
| --- | --- |
| `vespera-desktop-472cfe4.zip` | `2EB0403719D8A1082111CD8F34985F5951A58AA6D1B35D22AB32D8D2061CE9FE` |
| `vespera-0.1.2.msi` | `E86DA87F49255AEDE3E0CBD45C1F66AB7E408DCDBDC8F5D3EF4B2511FC3EA2CE` |
| `Vespera-Setup-0.1.2.exe` | `F741AD6F39192F44FB3E65951913E7F1489BEAF98B0E3B79D384E2DE4FB9A315` |

From a different working directory, the extracted launcher opened Home twice
with a scratch profile and each process exited after a close request. The
extracted Atlas Machine view reported a 7.41 ms p95 application update interval
over 60 focused samples after 120 warmup updates at 1200×800 on the RTX 4070
Laptop GPU via Vulkan. These are update intervals. Interactive acceptance for
the remaining Atlas views and workflow authoring, an installation lifecycle for
these exact artifacts, and signing remain open.

### Validated Windows x64 candidate

The candidate artifacts below are pinned to their recorded source revision.
Later source checkpoints add resumable SDK subscriptions, session-bound desktop
cancellation, explicit Atlas TLS trust and prompt broker-observer shutdown.
Those changes are not included in these ZIP/MSI/Setup files. Build from current
source to use them; a replacement optimized package must repeat the artifact,
installation and graphical acceptance checks before inheriting candidate status.

The optimized `0.1.2` internal candidate was built from `46819b7886db4cf078c77a588e28c9dad08b6d4e`. The portable ZIP verified all 45 manifest entries after extraction, including the owner-approved license. Administrative MSI extraction matched all 13 payload files against the stage. WiX extracted the Setup bundle and confirmed that its embedded MSI matched the standalone MSI byte for byte. The MSI and Setup are unsigned.

| Internal artifact | SHA-256 |
| --- | --- |
| `vespera-0.1.2-rc2-windows-x64.zip` | `D7F2966DDE6915CE049A76850BA79722F7805AD72FBC4F61409794A26C067A83` |
| `vespera-0.1.2.msi` | `54815DE98D99E30DC9C940AC3FF0E4BB1540441304028474FD8B2DD26343D643` |
| `Vespera-Setup-0.1.2.exe` | `BF85A44DE29D4F4B5F577ED77BDC67D03470A7769026BF5FF77C71E341E1CDEB` |

The extracted CLI passed configuration validation and host diagnostics. It read a temporary generic Windows credential, authenticated to a loopback fixture, and rejected a case-changed endpoint before connecting. The CLI also reopened the retained three-step desktop workflow. The packaged launcher opened Home from an unrelated working directory. The packaged Atlas rendered Machine on the RTX 4070 Laptop GPU through Vulkan and reported a focused 60-update sample after 120 warmup updates at 1200×800.

In the portable recovery rehearsal, the optimized `0.1.1` CLI created a world at revision 8. The `0.1.2` CLI advanced a copy to revision 9 and replayed revision 8. The older CLI reopened the original backup at revision 8, and its hash stayed unchanged. The hosted Windows job at the `0.1.2` version checkpoint passed the per-user `0.1.1` to `0.1.2` lifecycle with debug MSI fixtures, including downgrade rejection, notice hashes, and restoration of the saved world. [Hosted lifecycle](https://github.com/ShamelesAbyss/Vespera/actions/runs/42531557849/job/128886658103) · [Candidate source CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42532064448)

The installer rehearsal also injects a failure into a temporary WiX fixture after successful removal of the old product and before installation finalization. In a separate local per-user fixture, Windows Installer returned `1603` and restored the prior product registration and distinct prior license payload. The restored CLI opened the byte-identical revision 3 world. A subsequent clean upgrade and manual restore also passed. [Rollback checkpoint](https://github.com/ShamelesAbyss/Vespera/commit/140f712316c4dcd5731e9fc7872788c77fde2c99)

### Earlier Windows x64 candidate evidence

The optimized `0.1.1` archive was built from `3d232672abacbfac1f0467cbbc5c64e58157702d`. All 41 manifest entries verified after fresh extraction. The packaged CLI passed configuration validation and host diagnostics. Windows Installer's administrative extraction matched all 10 staged files carried by the MSI.

| Internal artifact | SHA-256 |
| --- | --- |
| `vespera-0.1.1-rc2-windows-x64.zip` | `21896DD54F7E7527EA857400FF79AE9803974E18667A58C1F6603A66F1A0BF78` |
| `vespera-0.1.1.msi` | `739B1293CB0877EA524D11957A808FA5B9E8DC17DB4132900303161FF727B46B` |
| `Vespera-Setup-0.1.1.exe` | `2E776B571A69C71F27D6D858EF4AF2E72AD1666BB86A9AD09BED399E95B35CE3` |

The Windows runner installed a per-user MSI fixture at `0.1.0`, upgraded to `0.1.1`, verified the downgrade guard, then removed the candidate and restored the earlier fixture. The saved world stayed byte-identical and reopened at revision 3. That rehearsal used debug payloads built in the runner. The local portable rollback separately reopened an actual `0.1.0` backup with the earlier binary. Both CI jobs passed at the archive's source revision. [Windows CI evidence](https://github.com/ShamelesAbyss/Vespera/actions/runs/42484887677/job/128744494799)

The final optimized archive opened all four Atlas views. At 1200×800 on an RTX 4070 Laptop GPU with Vulkan and FIFO presentation, p95 application update intervals were 7.81 ms for Machine, 7.50 ms for Intelligence Network, 7.32 ms for Swarm, and 7.35 ms for Living World. Each sample used 120 focused updates after 120 warmup updates. Living World committed revision 97, displayed revision 96 in replay, and reopened the live revision through the packaged CLI. These measurements cover steady views on one adapter.

The Windows walkthrough also covered local model discovery and registration, a live Ollama task, delegated workflow execution, and retained workflow history. Additional graphics environments, deployment between separate physical hosts, signing, independent legal review, and artifact-specific distribution notices remain release gates. [Development log](DEVELOPMENT_LOG.md)

### Linux desktop validation

The graphical build uses X11. It requires an available X11 display, the `libxkbcommon-x11` runtime and a wgpu adapter. On Ubuntu 24.04, the `libxkbcommon-x11-0` package supplies the keyboard runtime. The viewer reports a missing `DISPLAY` or keyboard runtime before initializing the window. The `vespera` CLI is the headless entry point.

The Ubuntu 24.04 WSLg rehearsal built the Linux graphics executables and displayed Living World through Mesa llvmpipe software Vulkan. It opened a copy of the Windows world at revision 97, committed revision 98, and replayed revision 97. The test used an isolated copy of the X11 keyboard libraries. Hardware-accelerated Linux graphics and other desktop environments remain separate qualification gates.
