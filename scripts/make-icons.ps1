# Generates the app and tray icons in assets/. The .ico files are build output of this
# script: edit the drawing here and re-run it instead of editing them by hand.
# Usage: powershell -ExecutionPolicy Bypass -File scripts/make-icons.ps1
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$out = Join-Path $PSScriptRoot '..\assets'
New-Item -ItemType Directory -Force $out | Out-Null

$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$cyan = [Drawing.Color]::FromArgb(255, 0x00, 0xE5, 0xFF)
$pink = [Drawing.Color]::FromArgb(255, 0xFF, 0x3D, 0x7F)

function New-RoundRect([single]$x, [single]$y, [single]$w, [single]$h, [single]$r) {
    $p = New-Object Drawing.Drawing2D.GraphicsPath
    $d = 2 * $r
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    $p
}

# Draws one frame. All geometry is in fractions of the icon size, so every
# size is drawn natively instead of being scaled from a big bitmap.
function New-Frame([int]$n, [Drawing.Color]$accent, [bool]$open) {
    $bmp = New-Object Drawing.Bitmap $n, $n, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $s = [single]$n

    # Dark tile, so the icon reads on light and dark taskbars alike.
    $bg = New-RoundRect 0 0 $s $s ($s * 0.22)
    $g.FillPath((New-Object Drawing.SolidBrush ([Drawing.Color]::FromArgb(255, 0x12, 0x14, 0x1C))), $bg)

    # The glowing border, echoing the on-screen glow. A soft wide pass, then a crisp one.
    $inset = $s * 0.09
    $ring = New-RoundRect $inset $inset ($s - 2 * $inset) ($s - 2 * $inset) ($s * 0.15)
    if ($n -ge 24) {
        $soft = New-Object Drawing.Pen ([Drawing.Color]::FromArgb(60, $accent)), ([Math]::Max(2, $s * 0.11))
        $g.DrawPath($soft, $ring)
    }
    $g.DrawPath((New-Object Drawing.Pen $accent, ([Math]::Max(1.25, $s * 0.05))), $ring)

    # Padlock, enlarged at tray sizes where the ring would otherwise dwarf it.
    if ($n -lt 24) {
        $g.TranslateTransform(($s * 0.5), ($s * 0.55))
        $g.ScaleTransform(1.2, 1.2)
        $g.TranslateTransform((-$s * 0.5), (-$s * 0.55))
    }
    $white = [Drawing.Color]::FromArgb(255, 0xF4, 0xF6, 0xFA)
    $shackle = New-Object Drawing.Pen $white, ($s * 0.085)
    $shackle.StartCap = 'Flat'; $shackle.EndCap = 'Flat'
    $cx = $s * 0.5; $r = $s * 0.125; $top = $s * 0.40; $bodyTop = $s * 0.48
    $lift = if ($open) { $s * $(if ($n -lt 24) { 0.05 } else { 0.075 }) } else { 0 }
    $path = New-Object Drawing.Drawing2D.GraphicsPath
    $leftEnd = if ($open) { $top - $lift + $s * 0.03 } else { $bodyTop }
    $path.AddLine(($cx - $r), $leftEnd, ($cx - $r), ($top - $lift))
    $path.AddArc(($cx - $r), ($top - $lift - $r), (2 * $r), (2 * $r), 180, 180)
    $path.AddLine(($cx + $r), ($top - $lift), ($cx + $r), $bodyTop)
    $g.DrawPath($shackle, $path)

    $body = New-RoundRect ($s * 0.29) $bodyTop ($s * 0.42) ($s * 0.31) ($s * 0.06)
    $g.FillPath((New-Object Drawing.SolidBrush $white), $body)
    if ($n -ge 20) {
        $hole = New-Object Drawing.SolidBrush ([Drawing.Color]::FromArgb(255, 0x12, 0x14, 0x1C))
        $k = $s * 0.045
        $g.FillEllipse($hole, ($cx - $k), ($s * 0.585), (2 * $k), (2 * $k))
        $g.FillRectangle($hole, ($cx - $k * 0.45), ($s * 0.62), ($k * 0.9), ($s * 0.08))
    }
    $g.Dispose()
    $bmp
}

# Writes an .ico whose frames are PNGs, which Windows has read since Vista.
function Save-Ico([string]$path, [Drawing.Color]$accent, [bool]$open) {
    $pngs = foreach ($n in $sizes) {
        $bmp = New-Frame $n $accent $open
        $ms = New-Object IO.MemoryStream
        $bmp.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        , $ms.ToArray()
    }
    $f = New-Object IO.MemoryStream
    $w = New-Object IO.BinaryWriter $f
    $w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $n = $sizes[$i]; $dim = if ($n -ge 256) { 0 } else { $n }
        $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
        $w.Write([uint16]1); $w.Write([uint16]32)
        $w.Write([uint32]$pngs[$i].Length); $w.Write([uint32]$offset)
        $offset += $pngs[$i].Length
    }
    foreach ($p in $pngs) { $w.Write($p) }
    $w.Flush()
    [IO.File]::WriteAllBytes($path, $f.ToArray())
    Write-Host "wrote $path"
}

Save-Ico (Join-Path $out 'app.ico') $cyan $false
Save-Ico (Join-Path $out 'tray-locked.ico') $pink $false
Save-Ico (Join-Path $out 'tray-unlocked.ico') $cyan $true

# Preview sheet for eyeballing every size on light and dark backgrounds.
if ($args -contains '-Preview') {
    $sheet = New-Object Drawing.Bitmap 760, 400
    $g = [Drawing.Graphics]::FromImage($sheet)
    $g.Clear([Drawing.Color]::FromArgb(255, 243, 243, 243))
    $g.FillRectangle((New-Object Drawing.SolidBrush ([Drawing.Color]::FromArgb(255, 32, 32, 32))), 0, 200, 760, 200)
    $variants = @(@($cyan, $false), @($pink, $false), @($cyan, $true))
    foreach ($band in 0, 200) {
        $x = 10
        foreach ($v in $variants) {
            foreach ($n in 16, 24, 32, 48) {
                $b = New-Frame $n $v[0] $v[1]; $g.DrawImageUnscaled($b, $x, $band + 10); $x += $n + 6; $b.Dispose()
            }
            $x += 10
        }
        $x = 10
        foreach ($v in $variants) {
            $b = New-Frame 128 $v[0] $v[1]; $g.DrawImageUnscaled($b, $x, $band + 66); $x += 150; $b.Dispose()
        }
    }
    # Tray sizes magnified 5x with hard pixels, to judge them pixel by pixel.
    $g.InterpolationMode = 'NearestNeighbor'; $g.PixelOffsetMode = 'Half'
    $x = 470
    foreach ($v in $variants) {
        $y = 10
        foreach ($n in 16, 20, 24) {
            $b = New-Frame $n $v[0] $v[1]; $g.DrawImage($b, $x, $y, $n * 5, $n * 5); $y += $n * 5 + 6; $b.Dispose()
        }
        $x += 100
    }
    $sheet.Save((Join-Path $env:TEMP 'dums-icons.png'))
    Write-Host "preview $(Join-Path $env:TEMP 'dums-icons.png')"
}
