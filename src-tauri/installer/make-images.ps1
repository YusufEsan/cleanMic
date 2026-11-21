# NSIS kurulum sihirbazı görsellerini (BMP) üretir: başlık 150x57, yan panel 164x314.
# Çalıştırma: powershell -ExecutionPolicy Bypass -File src-tauri\installer\make-images.ps1
Add-Type -AssemblyName System.Drawing
$dir = $PSScriptRoot
$bg = [System.Drawing.Color]::FromArgb(23, 20, 17)      # #171411
$panel = [System.Drawing.Color]::FromArgb(36, 32, 27)   # #24201B
$cream = [System.Drawing.Color]::FromArgb(242, 232, 213)# #F2E8D5
$hot = [System.Drawing.Color]::FromArgb(255, 138, 61)   # #FF8A3D
$muted = [System.Drawing.Color]::FromArgb(184, 172, 152)

function New-Canvas($w, $h) {
  $bmp = New-Object System.Drawing.Bitmap($w, $h, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = "AntiAlias"
  $g.TextRenderingHint = "AntiAliasGridFit"
  return @($bmp, $g)
}

function Draw-Mic($g, $cx, $cy, $s) {
  # basit mikrofon simgesi: kapsül + ayak
  $pen = New-Object System.Drawing.Pen($cream, [single](2.2 * $s))
  $pen.StartCap = "Round"; $pen.EndCap = "Round"
  $brush = New-Object System.Drawing.SolidBrush($cream)
  $g.FillEllipse($brush, [single]($cx - 5*$s), [single]($cy - 14*$s), [single](10*$s), [single](10*$s))
  $g.FillRectangle($brush, [single]($cx - 5*$s), [single]($cy - 9*$s), [single](10*$s), [single](9*$s))
  $g.FillEllipse($brush, [single]($cx - 5*$s), [single]($cy - 5*$s), [single](10*$s), [single](10*$s))
  $g.DrawArc($pen, [single]($cx - 10*$s), [single]($cy - 10*$s), [single](20*$s), [single](20*$s), 0, 180)
  $g.DrawLine($pen, [single]$cx, [single]($cy + 10*$s), [single]$cx, [single]($cy + 15*$s))
  $dot = New-Object System.Drawing.SolidBrush($hot)
  $g.FillEllipse($dot, [single]($cx + 9*$s), [single]($cy - 17*$s), [single](6*$s), [single](6*$s))
}

# ---- başlık (150x57): sağ üstte görünür
$c = New-Canvas 150 57; $bmp = $c[0]; $g = $c[1]
$g.Clear($panel)
Draw-Mic $g 126 30 1.0
$f1 = New-Object System.Drawing.Font("Segoe UI", 13, [System.Drawing.FontStyle]::Bold)
$g.DrawString("CleanMic", $f1, (New-Object System.Drawing.SolidBrush($cream)), 4, 10)
$f2 = New-Object System.Drawing.Font("Segoe UI", 7)
$g.DrawString("VOCAL PEDALBOARD", $f2, (New-Object System.Drawing.SolidBrush($muted)), 7, 34)
$bmp.Save((Join-Path $dir "header.bmp"), [System.Drawing.Imaging.ImageFormat]::Bmp)
$g.Dispose(); $bmp.Dispose()

# ---- yan panel (164x314): hoş geldin ve bitiş sayfalarında solda
$c = New-Canvas 164 314; $bmp = $c[0]; $g = $c[1]
$g.Clear($bg)
# pedal benzeri kutu
$rect = New-Object System.Drawing.Rectangle(22, 40, 120, 150)
$pb = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(122, 42, 30))
$g.FillRectangle($pb, $rect)
$g.DrawRectangle((New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(70, 255, 255, 255), 1)), $rect)
# iki düğme
foreach ($x in 52, 112) {
  $g.FillEllipse((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(28, 25, 21))), $x - 14, 58, 28, 28)
  $g.DrawLine((New-Object System.Drawing.Pen($cream, 2.5)), $x, 72, $x + 7, 63)
}
# LED + basmalı anahtar
$g.FillEllipse((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 59, 26))), 76, 104, 12, 12)
$g.FillEllipse((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(185, 176, 162))), 62, 128, 40, 40)
$g.DrawEllipse((New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(122, 112, 98), 3)), 62, 128, 40, 40)
$f1 = New-Object System.Drawing.Font("Segoe UI", 16, [System.Drawing.FontStyle]::Bold)
$g.DrawString("CleanMic", $f1, (New-Object System.Drawing.SolidBrush($cream)), 18, 210)
$tw = $g.MeasureString("CleanMic", $f1).Width
$g.FillEllipse((New-Object System.Drawing.SolidBrush($hot)), [single](18 + $tw - 7), 230, 6, 6)
$f2 = New-Object System.Drawing.Font("Segoe UI", 8)
$sf = New-Object System.Drawing.StringFormat
$g.DrawString("Temiz mikrofon,`neğlenceli efektler.", $f2, (New-Object System.Drawing.SolidBrush($muted)), (New-Object System.Drawing.RectangleF(20, 246, 130, 40)), $sf)
$bmp.Save((Join-Path $dir "sidebar.bmp"), [System.Drawing.Imaging.ImageFormat]::Bmp)
$g.Dispose(); $bmp.Dispose()
"ok"
