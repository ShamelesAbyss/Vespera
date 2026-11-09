# Build an unsigned x64 MSIX. Use an independent identity for local testing.
[CmdletBinding(DefaultParameterSetName = 'Store')]
param(
    [Parameter(Mandatory)][string]$Output,
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory, ParameterSetName = 'Store')][string]$IdentityFile,
    [Parameter(Mandatory, ParameterSetName = 'Development')][switch]$Development
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$outputPath = [IO.Path]::GetFullPath($Output, (Get-Location).ProviderPath)
if (Test-Path -LiteralPath $outputPath) { throw 'The MSIX output directory must be new.' }
if (!(Test-Path -LiteralPath (Split-Path -Parent $outputPath) -PathType Container)) {
    throw 'The parent of the output directory must exist.'
}
if ($Version -notmatch '^[1-9]\d{0,4}\.(?:0|[1-9]\d{0,4})\.(?:0|[1-9]\d{0,4})\.0$' -or
    @($Version.Split('.') | Where-Object { [int]$_ -gt 65535 }).Count) {
    throw 'Supply a Store package version in the form major.minor.build.0 with major greater than zero.'
}

$devName = 'Vespera.NexusStudio.Development'
$devPublisher = 'CN=Vespera Nexus Studio Local Development'
$displayName = 'Vespera Nexus Studio'
if ($Development) {
    $identity = @{ name = $devName; publisher = $devPublisher; publisherDisplayName = 'Vespera Local Development' }
    $displayName += ' (Development)'
} else {
    $identity = Get-Content -LiteralPath $IdentityFile -Raw | ConvertFrom-Json -AsHashtable
    if ($identity.storeProductId -cne '9PNV5XMS74FW' -or
        $identity.name -isnot [string] -or $identity.name -notmatch '^[A-Za-z0-9.-]{3,50}$' -or
        $identity.name -eq $devName -or
        $identity.publisher -isnot [string] -or $identity.publisher -eq $devPublisher -or
        [string]::IsNullOrWhiteSpace($identity.publisher) -or
        $identity.publisherDisplayName -isnot [string] -or
        [string]::IsNullOrWhiteSpace($identity.publisherDisplayName) -or
        $identity.packageFamilyName -isnot [string] -or
        $identity.packageFamilyName -cnotmatch ('^' + [regex]::Escape($identity.name) + '_[0-9a-z]{13}$')) {
        throw 'Supply the exact identity values from the Vespera Nexus Studio Product identity page.'
    }
}
$null = [Security.Cryptography.X509Certificates.X500DistinguishedName]::new($identity.publisher)

# Ask Windows to derive the family instead of maintaining a publisher hash algorithm.
if (!('VesperaStore.PackageIdentity' -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace VesperaStore {
    public static class PackageIdentity {
        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct PackageId {
            public uint Reserved;
            public uint ProcessorArchitecture;
            public ulong Version;
            public string Name;
            public string Publisher;
            public string ResourceId;
            public string PublisherId;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
        private static extern int PackageFamilyNameFromId(ref PackageId id, ref uint length, StringBuilder result);

        public static string FamilyName(string name, string publisher) {
            var id = new PackageId { Name = name, Publisher = publisher };
            var result = new StringBuilder(65);
            uint length = (uint)result.Capacity;
            int error = PackageFamilyNameFromId(ref id, ref length, result);
            if (error != 0) {
                throw new Win32Exception(error);
            }
            return result.ToString();
        }
    }
}
"@
}
$family = [VesperaStore.PackageIdentity]::FamilyName($identity.name, $identity.publisher)
if (!$Development -and ![string]::Equals($family, $identity.packageFamilyName, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The supplied package family does not match the Windows identity for the supplied name and publisher.'
}

$sdkBin = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
$makeAppx = Get-ChildItem -LiteralPath $sdkBin -Directory |
    Where-Object { $_.Name -match '^10\.0\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64/makeappx.exe' } |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
if (!$makeAppx) { throw 'Install the Windows SDK with x64 MakeAppx.' }
$manifestTool = Join-Path (Split-Path -Parent $makeAppx) 'mt.exe'
if (!(Test-Path -LiteralPath $manifestTool -PathType Leaf)) {
    throw 'Install the Windows SDK with x64 Manifest Tool.'
}

Push-Location -LiteralPath $root
try {
    $revision = (git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or !$revision) { throw 'Could not read the Vespera source revision.' }
    $initialStatus = git status --porcelain --untracked-files=normal
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the Vespera source tree.' }
    if (!$Development -and $initialStatus) {
        throw 'Build Store candidates from a clean checkout. Keep identity input outside the checkout or under target.'
    }
    $events = cargo build --locked --release -p vespera-view --features graphics,store --bin vespera-view --message-format=json-render-diagnostics
    if ($LASTEXITCODE -ne 0) { throw 'Could not build the optimized Vespera Store viewer.' }
    $built = $events | ForEach-Object { ConvertFrom-Json -InputObject $_ } |
        Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'vespera-view' -and $_.executable } |
        Select-Object -Last 1
    if (!$built -or !(Test-Path -LiteralPath $built.executable -PathType Leaf)) {
        throw 'Cargo did not report the Store viewer executable.'
    }
    $finalStatus = git status --porcelain --untracked-files=normal
    if ($LASTEXITCODE -ne 0 -or (git rev-parse HEAD).Trim() -ne $revision -or
        (!$Development -and $finalStatus)) {
        throw 'Source changed during packaging.'
    }
    $stream = [IO.File]::OpenRead($built.executable)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5A4D) { throw 'The viewer is not a Windows PE executable.' }
        $stream.Position = 0x3C
        $stream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw 'The viewer executable must target Windows x64.'
        }
    } finally { $reader.Dispose() }

    $payload = Join-Path $outputPath 'payload'
    $brand = Join-Path $payload 'Branding'
    New-Item -ItemType Directory -Path $brand,(Join-Path $payload 'bin'),(Join-Path $payload 'licenses'),(Join-Path $payload 'config'),(Join-Path $payload 'assets/world/space-base') | Out-Null
    $viewer = Join-Path $payload 'bin/vespera-view.exe'
    Copy-Item -LiteralPath $built.executable -Destination $viewer
    $embeddedManifest = Join-Path $outputPath 'viewer-win32.manifest'
    & $manifestTool "-inputresource:$viewer;#1" "-out:$embeddedManifest"
    if ($LASTEXITCODE -ne 0) { throw 'The Store viewer has no readable Win32 application manifest.' }
    [xml]$viewerManifest = Get-Content -LiteralPath $embeddedManifest -Raw
    $dpi = $viewerManifest.SelectSingleNode("//*[local-name()='dpiAwareness' and namespace-uri()='http://schemas.microsoft.com/SMI/2016/WindowsSettings']")
    if (!$dpi -or $dpi.InnerText.Trim() -cne 'PerMonitorV2') {
        throw 'The Store viewer must declare PerMonitorV2 DPI awareness.'
    }
    Copy-Item -LiteralPath 'LICENSE','THIRD_PARTY_NOTICES.md' -Destination $payload
    Copy-Item -LiteralPath 'licenses/FiraMono-LICENSE.txt','licenses/windows-x64-cargo-inventory.txt' -Destination (Join-Path $payload 'licenses')
    Copy-Item -LiteralPath 'assets/world/ATTRIBUTION.md' -Destination (Join-Path $payload 'assets/world')
    Copy-Item -LiteralPath 'assets/world/space-base/LICENSE.txt' -Destination (Join-Path $payload 'assets/world/space-base')
    Copy-Item -LiteralPath 'config/local.toml','config/world.example.json','config/workflow.example.json' -Destination (Join-Path $payload 'config')

    Add-Type -AssemblyName System.Drawing
    $original = [Drawing.Image]::FromFile((Resolve-Path -LiteralPath 'assets/branding/vespera.png').Path)
    try {
        if ($original.Width -ne $original.Height) { throw 'The official branding source must be square.' }
        foreach ($asset in @(
            @((Join-Path $brand 'StoreLogo.png'), 50),
            @((Join-Path $brand 'Square150x150Logo.png'), 150),
            @((Join-Path $brand 'Square44x44Logo.png'), 44),
            @((Join-Path $outputPath 'StoreListingTile.png'), 300)
        )) {
            $size = $asset[1]
            $bitmap = [Drawing.Bitmap]::new($size, $size)
            $graphics = [Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.DrawImage($original, 0, 0, $size, $size)
                $bitmap.Save($asset[0], [Drawing.Imaging.ImageFormat]::Png)
            } finally {
                $graphics.Dispose()
                $bitmap.Dispose()
            }
        }
    } finally { $original.Dispose() }

    [xml]$manifest = Get-Content -LiteralPath 'packaging/windows/store/AppxManifest.xml' -Raw
    $manifest.Package.Identity.SetAttribute('Name', $identity.name)
    $manifest.Package.Identity.SetAttribute('Publisher', $identity.publisher)
    $manifest.Package.Identity.SetAttribute('Version', $Version)
    $manifest.Package.Properties.GetElementsByTagName('DisplayName')[0].InnerText = $displayName
    $manifest.Package.Properties.GetElementsByTagName('PublisherDisplayName')[0].InnerText = $identity.publisherDisplayName
    $manifest.Package.Applications.Application.VisualElements.SetAttribute('DisplayName', $displayName)
    $manifest.Save((Join-Path $payload 'AppxManifest.xml'))

    $label = if ($Development) { 'DEVELOPMENT-UNSIGNED' } else { 'STORE-UNSIGNED' }
    $package = Join-Path $outputPath "Vespera-Nexus-Studio-$Version-x64-$label.msix"
    & $makeAppx pack /d $payload /p $package /h SHA256
    if ($LASTEXITCODE -ne 0) { throw 'MakeAppx rejected the package.' }
    $verified = Join-Path $outputPath 'verified'
    & $makeAppx unpack /p $package /d $verified
    if ($LASTEXITCODE -ne 0) { throw 'MakeAppx could not unpack the result.' }
    foreach ($file in Get-ChildItem -LiteralPath $payload -Recurse -File) {
        $relative = [IO.Path]::GetRelativePath($payload, $file.FullName)
        $unpacked = Join-Path $verified $relative
        if ((Get-FileHash -LiteralPath $file.FullName).Hash -ne (Get-FileHash -LiteralPath $unpacked).Hash) {
            throw "MSIX payload verification failed for $relative."
        }
    }
    $hash = (Get-FileHash -LiteralPath $package).Hash.ToLowerInvariant()
    $lockHash = (Get-FileHash -LiteralPath 'Cargo.lock').Hash.ToLowerInvariant()
    [ordered]@{
        mode = $label
        sourceRevision = $revision
        cleanSource = !$initialStatus -and !$finalStatus
        cargoLockSha256 = $lockHash
        packageVersion = $Version
        architecture = 'x64'
        packageName = $identity.name
        packageFamilyName = $family
        expectedPackageFamilyName = $identity.packageFamilyName
        packageSha256 = $hash
        viewerSha256 = (Get-FileHash -LiteralPath $viewer).Hash.ToLowerInvariant()
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $outputPath 'build-evidence.json')
    Write-Output "$hash  $package"
} finally { Pop-Location }
