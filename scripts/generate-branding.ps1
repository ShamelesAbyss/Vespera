$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$brand = Join-Path $PSScriptRoot '../assets/branding'
$source = [System.Drawing.Image]::FromFile((Join-Path $brand 'vespera-original.jpg'))
try {
    if ($source.Width -ne $source.Height) { throw 'The canonical Vespera image must be square.' }
    $pngs = @{}
    foreach ($size in @(16, 20, 24, 32, 40, 48, 64, 128, 256, 512, 1024)) {
        $bitmap = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $stream = [System.IO.MemoryStream]::new()
        try {
            $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $graphics.DrawImage($source, 0, 0, $size, $size)
            $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
            $pngs[$size] = $stream.ToArray()
        } finally {
            $stream.Dispose()
            $graphics.Dispose()
            $bitmap.Dispose()
        }
    }
} finally {
    $source.Dispose()
}

[System.IO.File]::WriteAllBytes((Join-Path $brand 'vespera.png'), $pngs[256])

$sizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)
$ico = [System.IO.BinaryWriter]::new([System.IO.File]::Create((Join-Path $brand 'vespera.ico')))
try {
    $ico.Write([uint16]0)
    $ico.Write([uint16]1)
    $ico.Write([uint16]$sizes.Length)
    $offset = 6 + 16 * $sizes.Length
    foreach ($size in $sizes) {
        $edge = [byte]($size % 256)
        $ico.Write($edge)
        $ico.Write($edge)
        $ico.Write([uint16]0)
        $ico.Write([uint16]1)
        $ico.Write([uint16]32)
        $ico.Write([uint32]$pngs[$size].Length)
        $ico.Write([uint32]$offset)
        $offset += $pngs[$size].Length
    }
    foreach ($size in $sizes) { $ico.Write([byte[]]$pngs[$size]) }
} finally {
    $ico.Dispose()
}

function Write-BigEndian32($stream, [int]$number) {
    $data = [BitConverter]::GetBytes($number)
    if ([BitConverter]::IsLittleEndian) { [Array]::Reverse($data) }
    $stream.Write($data, 0, $data.Length)
}

$mac = [ordered]@{ icp4 = 16; icp5 = 32; icp6 = 64; ic07 = 128; ic08 = 256; ic09 = 512; ic10 = 1024 }
$icns = [System.IO.File]::Create((Join-Path $brand 'vespera.icns'))
try {
    $total = 8
    foreach ($size in $mac.Values) { $total += 8 + $pngs[$size].Length }
    $icns.Write([Text.Encoding]::ASCII.GetBytes('icns'), 0, 4)
    Write-BigEndian32 $icns $total
    foreach ($type in $mac.Keys) {
        $data = $pngs[$mac[$type]]
        $icns.Write([Text.Encoding]::ASCII.GetBytes($type), 0, 4)
        Write-BigEndian32 $icns (8 + $data.Length)
        $icns.Write($data, 0, $data.Length)
    }
} finally {
    $icns.Dispose()
}
