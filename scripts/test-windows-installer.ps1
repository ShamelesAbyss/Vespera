# Run only on an isolated Windows GitHub Actions runner.
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or !$env:RUNNER_TEMP) {
    throw 'This rehearsal requires a disposable GitHub Actions Windows runner.'
}

$installed = Join-Path $env:LOCALAPPDATA 'Programs/Vespera'
$data = Join-Path $env:LOCALAPPDATA 'Vespera'
if ((Test-Path -LiteralPath $installed) -or (Test-Path -LiteralPath $data)) {
    throw 'The runner already has Vespera binaries or data.'
}

$fixture = Join-Path $env:RUNNER_TEMP 'vespera-installer'
$stage = Join-Path $fixture 'stage'
New-Item -ItemType Directory -Path "$stage/bin","$stage/assets" | Out-Null
Copy-Item -LiteralPath @('target/debug/vespera.exe','target/debug/vespera-view.exe','target/debug/vespera-launcher.exe') -Destination "$stage/bin"
Copy-Item -LiteralPath 'assets/branding','assets/world' -Destination "$stage/assets" -Recurse
Copy-Item -LiteralPath 'LICENSE' -Destination $stage

$env:DOTNET_ROLL_FORWARD = 'Major'
dotnet tool install wix --version 5.0.2 --tool-path "$fixture/tools"
if ($LASTEXITCODE -ne 0) { throw 'Could not install the pinned WiX tool.' }
$version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"$').Matches[0].Groups[1].Value
$previous = '0.1.0'
if ([version]$version -le [version]$previous) { throw 'Expected a newer installer version.' }

foreach ($productVersion in $previous,$version) {
    $package = Join-Path $fixture "$productVersion.msi"
    $wixArguments = @('build','-arch','x64','-d',"Stage=$stage",'-d',"Version=$productVersion",
        '-o',$package,'packaging/windows/vespera.wxs')
    & "$fixture/tools/wix.exe" @wixArguments
    if ($LASTEXITCODE -ne 0) { throw "MSI build failed for $productVersion." }
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

Invoke-Installer /i $previous 'install'
Assert-Version $previous
$launcher = [IO.Path]::GetFullPath((Join-Path $installed 'bin/vespera-launcher.exe'))
$shell = New-Object -ComObject WScript.Shell
foreach ($location in 'Desktop','Programs') {
    $link = Join-Path ([Environment]::GetFolderPath($location)) 'Vespera.lnk'
    if (!(Test-Path -LiteralPath $link) -or $shell.CreateShortcut($link).TargetPath -ne $launcher) {
        throw "Invalid $location shortcut."
    }
}

New-Item -ItemType Directory -Path $data | Out-Null
$world = Join-Path $data 'installer-test.sqlite3'
$cli = Join-Path $installed 'bin/vespera.exe'
& $cli world --database $world init --spec config/world.example.json | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Installed CLI could not create the fixture world.' }
& $cli world --database $world advance --ticks 3 | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Installed CLI could not advance the fixture world.' }
$worldHash = (Get-FileHash -LiteralPath $world).Hash

Invoke-Installer /i $version 'upgrade'
Assert-Version $version
Invoke-Installer /i $previous 'downgrade' 1603
Assert-Version $version
if (!(Select-String -Path "$fixture/downgrade.log" -Pattern 'A newer version of Vespera is already installed' -Quiet)) {
    throw 'The older package did not report the downgrade guard.'
}
Invoke-Installer /x $version 'remove-candidate'
Invoke-Installer /i $previous 'restore-previous'
Assert-Version $previous
if ((Get-FileHash -LiteralPath $world).Hash -ne $worldHash) {
    throw 'Installer lifecycle changed the saved world bytes.'
}
$state = & $cli world --database $world inspect | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $state.revision -ne 3) {
    throw 'The restored installation could not read the saved world.'
}
Invoke-Installer /x $previous 'remove-fixture'
if (!(Test-Path -LiteralPath $world)) { throw 'Uninstall removed the fixture world.' }
Write-Output "Verified per-user MSI install, upgrade to $version, downgrade rejection, and restore of $previous with the committed world preserved."
