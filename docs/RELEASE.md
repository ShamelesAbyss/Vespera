# Local release validation

Vespera has an internal release candidate workflow. Public distribution requires the owner’s licensing decision and a review of third-party notices. The current project license reserves redistribution rights. [Project license](../LICENSE)

## Build and stage

Use the pinned Rust toolchain, Git and the platform build tools. Start from a commit with a clean tracked worktree. Run the packager from the workspace root. [Build requirements](../README.md#requirements)

```powershell
cargo run --locked -p vespera-app --example package-local -- --output target/vespera-local
```

The same Cargo command works in a POSIX shell. Add `--offline` after the final `--` to require a cached dependency set for the release build. To keep the entire invocation offline, also put `--offline` before `-p`. Add `--headless` after the final `--` to stage the CLI alone. Use a fresh output directory with an existing parent.

The tool builds the release executables with the lockfile and stages the requested binaries, tracked configuration examples, licenses, branding, screenshots and operator documentation. It copies tracked materials from the recorded Git revision. `manifest.json` lists the source revision, Cargo.lock hash and SHA-256 and byte size of each staged file. A staging error can leave a partial directory. Treat a directory without a manifest as incomplete.

On Windows, open `bin\vespera-launcher.exe` for the native desktop without a console. The graphics build requires the Windows SDK resource compiler. Home shows the official icon on its first rendered frame. Other screens use the same image in the shared header. The desktop also embeds the PNG and Windows icon resources, so it can render its branding from any working directory. The portable package includes the unchanged original and the derived PNG, multi-resolution ICO and ICNS. The ICNS is available for future macOS app packaging. Run `./scripts/generate-branding.ps1` on Windows to regenerate derivatives from the canonical JPEG.

## Verify and start

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

Use a higher three-component product version for each MSI upgrade. The `0.1.1` candidate supersedes the earlier `0.1.0` rehearsal. Old-product removal is part of the Windows Installer transaction. Use the same committed candidate for both the MSI and its Setup bundle.

## Upgrade and recovery

Stage the new binaries in a fresh directory. Stop Vespera writers before making operator backups. Use `memory-admin backup` for the memory store. For stores without a Vespera backup command, preserve a consistent stopped-state SQLite copy, including associated WAL and SHM files when present. Keep each database with its own backup set.

Rehearse the new binaries on copies of the backed-up stores. Verify the catalog, tasks, workflow results and journal cursor ranges you rely on before switching the live paths. Broker migrations run on startup with `--broker-db`. Broker schema 3 adds durable mailboxes. Event schema 8 adds optional mailbox acknowledgement events and requires updated readers when enabled. [Broker recovery and compatibility](PROTOCOL.md#durable-broker-recovery)

For rollback, use the previous binaries with their pre-upgrade backups and compatible configuration. Preserve the upgraded stores separately for reconciliation. Reopening an upgraded live database with an older binary is only supported where the subsystem explicitly documents compatibility.

## Release evidence

- Record the Git revision, Windows or Linux architecture, build profile and package checksums.
- Verify the fresh-directory walkthrough, headless and graphics builds, provider matrix, permission checks and backup recovery.
- Regenerate the dependency graph and declared license inventory from the locked build. Review the actual artifacts, license texts and notices before distribution. [Dependency record](DEPENDENCIES.md)
- Record remaining compatibility and release gates in the milestone ledger. [Milestone ledger](MILESTONE_LEDGER.md)

The local Windows archive walkthrough verified 27 files, native Atlas on an RTX 4070 Laptop GPU and offline world replay. The broker rollback rehearsal used an actual prior schema 2 binary and a stopped-state copy including its SQLite sidecars. The prior binary refused the migrated store and reopened a restored backup. [Development log](DEVELOPMENT_LOG.md)
