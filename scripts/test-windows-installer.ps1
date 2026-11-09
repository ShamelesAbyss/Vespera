# Run only on an isolated Windows GitHub Actions runner.
param([switch]$Optimized)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or !$env:RUNNER_TEMP) {
    throw 'This rehearsal requires a disposable GitHub Actions Windows runner.'
}

$installed = Join-Path $env:LOCALAPPDATA 'Programs/Vespera'
$data = Join-Path $env:LOCALAPPDATA 'Vespera'
if ((Test-Path -LiteralPath $installed) -or (Test-Path -LiteralPath $data)) {
    throw 'The runner already has Vespera binaries or data.'
}

$fixture = Join-Path $env:RUNNER_TEMP 'vespera-installer'
$stage = Join-Path $fixture 'stage'
$previousStage = Join-Path $fixture 'previous-stage'
$authoring = 'packaging/windows/vespera.wxs'
$previousAuthoring = $authoring
$version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"$').Matches[0].Groups[1].Value
$previous = '0.1.1'
if ([version]$version -le [version]$previous) { throw 'Expected a newer installer version.' }

if ($Optimized) {
    New-Item -ItemType Directory -Path $fixture | Out-Null
    $priorRevision = '3d232672abacbfac1f0467cbbc5c64e58157702d'
    $priorSource = Join-Path $fixture 'previous-source'
    git -c core.longpaths=true worktree add --detach $priorSource $priorRevision
    if ($LASTEXITCODE -ne 0) { throw 'Could not open the qualified prior revision.' }
    $previousAuthoring = Join-Path $priorSource $authoring
    $env:CARGO_TARGET_DIR = Join-Path $fixture 'target'
    Push-Location -LiteralPath $priorSource
    try {
        cargo run --locked -p vespera-app --example package-local -- --output $previousStage
        if ($LASTEXITCODE -ne 0) { throw 'Could not build the optimized prior package.' }
    } finally {
        Pop-Location
    }
    cargo run --locked -p vespera-app --example package-local -- --output $stage
    if ($LASTEXITCODE -ne 0) { throw 'Could not build the optimized candidate.' }
    $currentRevision = (git rev-parse HEAD).Trim()
    foreach ($package in @(
        @{ Path = $previousStage; Revision = $priorRevision; Version = $previous },
        @{ Path = $stage; Revision = $currentRevision; Version = $version }
    )) {
        $manifest = Get-Content -LiteralPath (Join-Path $package.Path 'manifest.json') -Raw | ConvertFrom-Json
        if ($manifest.source_revision -ne $package.Revision -or $manifest.vespera_version -ne $package.Version -or
            !$manifest.graphics -or $manifest.build -ne 'cargo release, locked' -or
            $manifest.build_host_os -ne 'windows' -or $manifest.build_host_arch -ne 'x86_64') {
            throw 'Unexpected optimized package identity.'
        }
        cargo run --locked -p vespera-app --example verify-package -- --package $package.Path
        if ($LASTEXITCODE -ne 0) { throw "Could not verify package $($package.Version)." }
        Write-Output "Verified package $($package.Version) from $($package.Revision)."
    }
} else {
    New-Item -ItemType Directory -Path "$stage/bin","$stage/assets" | Out-Null
    Copy-Item -LiteralPath @('target/debug/vespera.exe','target/debug/vespera-view.exe','target/debug/vespera-launcher.exe') -Destination "$stage/bin"
    Copy-Item -LiteralPath 'assets/branding','assets/world' -Destination "$stage/assets" -Recurse
    Copy-Item -LiteralPath 'LICENSE','THIRD_PARTY_NOTICES.md' -Destination $stage
    Copy-Item -LiteralPath 'licenses' -Destination $stage -Recurse
    Copy-Item -LiteralPath $stage -Destination $previousStage -Recurse
    Add-Content -LiteralPath (Join-Path $previousStage 'LICENSE') -Value 'Prior MSI lifecycle fixture marker.'
}

$env:DOTNET_ROLL_FORWARD = 'Major'
dotnet tool install wix --version 5.0.2 --tool-path "$fixture/tools"
if ($LASTEXITCODE -ne 0) { throw 'Could not install the pinned WiX tool.' }

function Build-Msi($ProductStage, $ProductVersion, $Output, $Authoring) {
    $wixArguments = @('build','-arch','x64','-d',"Stage=$ProductStage",'-d',"Version=$ProductVersion",
        '-o',$Output,$Authoring)
    & "$fixture/tools/wix.exe" @wixArguments
    if ($LASTEXITCODE -ne 0) { throw "MSI build failed for $Output." }
}
$priorMsi = Join-Path $fixture "$previous.msi"
$candidateMsi = Join-Path $fixture "$version.msi"
Build-Msi $previousStage $previous $priorMsi $previousAuthoring
Build-Msi $stage $version $candidateMsi $authoring

# Add a Type 19 failure to temporary authoring, after old-product removal and before finalization.
$rollbackMessage = 'Vespera installer fixture abort after old product removal'
$rollbackAuthoring = Join-Path $fixture 'rollback-fault.wxs'
$failureAction = @"
    <CustomAction Id="VesperaRollbackFixture" Error="$rollbackMessage" />
    <InstallExecuteSequence>
      <Custom Action="VesperaRollbackFixture" Before="InstallFinalize" Condition="WIX_UPGRADE_DETECTED" />
    </InstallExecuteSequence>
  </Package>
"@
$source = Get-Content -LiteralPath $authoring -Raw
Set-Content -LiteralPath $rollbackAuthoring -Value $source.Replace('</Package>', $failureAction)
Build-Msi $stage $version (Join-Path $fixture 'rollback-fault.msi') $rollbackAuthoring

$bootstrapper = 'WixToolset.BootstrapperApplications.wixext'
& "$fixture/tools/wix.exe" extension add -g "$bootstrapper/5.0.2"
if ($LASTEXITCODE -ne 0) { throw 'Could not install the pinned WiX bootstrapper extension.' }
$setup = Join-Path $fixture "Vespera-Setup-$version.exe"
& "$fixture/tools/wix.exe" build -arch x64 -ext $bootstrapper -d "Stage=$stage" -d "Version=$version" `
    -d "Msi=$candidateMsi" -o $setup 'packaging/windows/setup.wxs'
if ($LASTEXITCODE -ne 0) { throw 'Setup bundle build failed.' }
if ($Optimized) {
    $extracted = Join-Path $fixture 'bundle'
    & "$fixture/tools/wix.exe" burn extract $setup -o $extracted
    if ($LASTEXITCODE -ne 0) { throw 'Setup extraction failed.' }
    $payload = @(Get-ChildItem -LiteralPath $extracted -Recurse -File)
    if ($payload.Count -ne 1 -or (Get-FileHash -LiteralPath $payload[0].FullName).Hash -ne
        (Get-FileHash -LiteralPath $candidateMsi).Hash) {
        throw 'Setup payload differs from the verified candidate MSI.'
    }
    Get-FileHash -LiteralPath $priorMsi,$candidateMsi,$setup | ForEach-Object {
        Write-Output "$($_.Hash.ToLowerInvariant())  $(Split-Path -Leaf $_.Path)"
    }
}

function Invoke-Setup($Operation, $Phase) {
    $log = Join-Path $fixture "$Phase.log"
    $setupArguments = '{0} /quiet /norestart /log "{1}"' -f $Operation, $log
    $process = Start-Process -FilePath $setup -ArgumentList $setupArguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        Get-Content -LiteralPath $log -Tail 45 -ErrorAction Continue
        throw "Setup phase $Phase returned $($process.ExitCode)."
    }
}

function Invoke-Installer($Operation, $ProductVersion, $Phase, $Expected = 0) {
    $log = Join-Path $fixture "$Phase.log"
    $package = (Resolve-Path -LiteralPath (Join-Path $fixture "$ProductVersion.msi")).Path
    $installerArguments = '{0} "{1}" /qn /norestart /l*v "{2}"' -f $Operation, $package, $log
    $process = Start-Process msiexec.exe -ArgumentList $installerArguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne $Expected) {
        Get-Content -LiteralPath $log -Tail 45
        throw "Installer phase $Phase returned $($process.ExitCode)."
    }
}

$msiInspector = New-Object -ComObject WindowsInstaller.Installer
$upgradeCode = '{81BA1438-9EAE-4707-AB97-66D09CAD81DD}'
function Assert-Version($Expected) {
    $products = $msiInspector.GetType().InvokeMember('RelatedProducts','GetProperty',$null,$msiInspector,@($upgradeCode))
    $count = $products.GetType().InvokeMember('Count','GetProperty',$null,$products,$null)
    if ($count -ne 1) {
        throw "Expected one installed product at version $Expected."
    }
    $product = $products.GetType().InvokeMember('Item','GetProperty',$null,$products,@(0))
    $actual = $msiInspector.GetType().InvokeMember('ProductInfo','GetProperty',$null,$msiInspector,@($product,'VersionString'))
    if ([version]$actual -ne [version]$Expected) { throw "Expected version $Expected, found $actual." }
}

function Assert-InstalledPayload($ExpectedStage, $ExpectedAuthoring) {
    $namespace = @{ w = 'http://wixtoolset.org/schemas/v4/wxs' }
    $expectedFiles = @(Select-Xml -LiteralPath $ExpectedAuthoring -XPath '//w:File' -Namespace $namespace)
    $files = @(Get-ChildItem -LiteralPath $installed -Recurse -File)
    if ($files.Count -ne $expectedFiles.Count) { throw 'Unexpected installed payload count.' }
    foreach ($file in $files) {
        $relative = [IO.Path]::GetRelativePath($installed, $file.FullName)
        $expected = Join-Path $ExpectedStage $relative
        if ((Get-FileHash -LiteralPath $file.FullName).Hash -ne (Get-FileHash -LiteralPath $expected).Hash) {
            throw "Installed payload differs from staged file $relative."
        }
    }
}

function Assert-Shortcuts {
    $launcher = [IO.Path]::GetFullPath((Join-Path $installed 'bin/vespera-launcher.exe'))
    $shell = New-Object -ComObject WScript.Shell
    foreach ($location in 'Desktop','Programs') {
        $link = Join-Path ([Environment]::GetFolderPath($location)) 'Vespera.lnk'
        if (!(Test-Path -LiteralPath $link) -or $shell.CreateShortcut($link).TargetPath -ne $launcher) {
            throw "Invalid $location shortcut."
        }
    }
}

if ($Optimized) {
    Invoke-Setup /install 'setup-clean-install'
    Assert-Version $version
    Assert-InstalledPayload $stage $authoring
    Assert-Shortcuts
    Invoke-Setup /uninstall 'setup-clean-remove'
    if (Test-Path -LiteralPath (Join-Path $installed 'bin/vespera.exe')) {
        throw 'The clean-install rehearsal left a candidate executable.'
    }
    Write-Output "Verified a clean Setup installation of $version."
}

Invoke-Installer /i $previous 'install'
Assert-Version $previous
Assert-InstalledPayload $previousStage $previousAuthoring
Assert-Shortcuts

New-Item -ItemType Directory -Path $data | Out-Null
$world = Join-Path $data 'installer-test.sqlite3'
$cli = Join-Path $installed 'bin/vespera.exe'
& $cli world --database $world init --spec config/world.example.json | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Installed CLI could not create the fixture world.' }
& $cli world --database $world advance --ticks 3 | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Installed CLI could not advance the fixture world.' }
$worldHash = (Get-FileHash -LiteralPath $world).Hash
if ($Optimized -and (& $cli --version) -ne "vespera $previous") {
    throw 'Installed prior CLI reports an unexpected version.'
}

Invoke-Installer /i 'rollback-fault' 'failed-upgrade' 1603
$failedUpgrade = Join-Path $fixture 'failed-upgrade.log'
$removal = Select-String -LiteralPath $failedUpgrade -Pattern 'Action ended .*RemoveExistingProducts\. Return value 1\.' | Select-Object -First 1
$failure = Select-String -LiteralPath $failedUpgrade -Pattern $rollbackMessage -SimpleMatch | Select-Object -First 1
if (!$removal -or !$failure -or $removal.LineNumber -ge $failure.LineNumber) {
    Get-Content -LiteralPath $failedUpgrade -Tail 60
    throw 'The fault did not run after successful old product removal.'
}
Assert-Version $previous
Assert-InstalledPayload $previousStage $previousAuthoring
if ((Get-FileHash -LiteralPath $world).Hash -ne $worldHash) {
    throw 'Failed upgrade changed the saved world bytes.'
}
$rollbackState = & $cli world --database $world inspect | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $rollbackState.revision -ne 3) {
    throw 'Automatic rollback did not restore the prior installation.'
}

Invoke-Setup /install 'setup-upgrade'
Assert-Version $version
Assert-InstalledPayload $stage $authoring
Assert-Shortcuts
if ($Optimized -and (& $cli --version) -ne "vespera $version") {
    throw 'Installed candidate CLI reports an unexpected version.'
}
if ((Get-FileHash -LiteralPath $world).Hash -ne $worldHash) {
    throw 'Successful upgrade changed the saved world bytes.'
}
$upgradedState = & $cli world --database $world inspect | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $upgradedState.revision -ne 3) {
    throw 'The upgraded installation could not read the saved world.'
}
Invoke-Installer /i $previous 'downgrade' 1603
Assert-Version $version
if (!(Select-String -Path "$fixture/downgrade.log" -Pattern 'A newer version of Vespera is already installed' -Quiet)) {
    throw 'The older package did not report the downgrade guard.'
}
Invoke-Setup /uninstall 'setup-remove-candidate'
Invoke-Installer /i $previous 'restore-previous'
Assert-Version $previous
Assert-InstalledPayload $previousStage $previousAuthoring
if ((Get-FileHash -LiteralPath $world).Hash -ne $worldHash) {
    throw 'Installer lifecycle changed the saved world bytes.'
}
$state = & $cli world --database $world inspect | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $state.revision -ne 3) {
    throw 'The restored installation could not read the saved world.'
}
Invoke-Installer /x $previous 'remove-fixture'
if (!(Test-Path -LiteralPath $world)) { throw 'Uninstall removed the fixture world.' }
Write-Output "Verified per-user MSI install, automatic rollback after a failed upgrade, Setup bundle upgrade to $version, downgrade rejection, Setup removal, and restore of $previous with the committed world preserved."
