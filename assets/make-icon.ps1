# Builds assets\icon.ico, the icon embedded in SparkyPDF.exe, from the artwork in
# assets\rtx-spark-laptop-pdf-icon.png. The artwork file itself is never changed.
#
#   powershell -ExecutionPolicy Bypass -File assets\make-icon.ps1
param([string]$Source = (Join-Path $PSScriptRoot 'rtx-spark-laptop-pdf-icon.png'))

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class IconArt {
    static int[] Read(Bitmap bmp, out BitmapData data) {
        data = bmp.LockBits(new Rectangle(0, 0, bmp.Width, bmp.Height), ImageLockMode.ReadWrite, PixelFormat.Format32bppArgb);
        var px = new int[bmp.Width * bmp.Height];
        Marshal.Copy(data.Scan0, px, 0, px.Length);
        return px;
    }

    // See-through areas that are completely surrounded by solid artwork are holes left
    // behind when a white background was removed (the white-hot centre of the spark).
    // They are put back as white. Returns { pixels changed, left, top, right, bottom }.
    public static int[] FillHoles(Bitmap bmp) {
        int w = bmp.Width, h = bmp.Height;
        BitmapData data;
        var px = Read(bmp, out data);
        var outside = new bool[px.Length];
        var todo = new Stack<int>();
        Action<int> visit = i => {
            if (!outside[i] && ((px[i] >> 24) & 255) < 250) { outside[i] = true; todo.Push(i); }
        };
        for (int x = 0; x < w; x++) { visit(x); visit((h - 1) * w + x); }
        for (int y = 0; y < h; y++) { visit(y * w); visit(y * w + w - 1); }
        while (todo.Count > 0) {
            int i = todo.Pop(), x = i % w, y = i / w;
            if (x > 0) visit(i - 1);
            if (x < w - 1) visit(i + 1);
            if (y > 0) visit(i - w);
            if (y < h - 1) visit(i + w);
        }
        var result = new int[] { 0, w, h, 0, 0 };
        for (int i = 0; i < px.Length; i++) {
            int a = (px[i] >> 24) & 255;
            if (a >= 250 || outside[i]) continue;
            int r = (px[i] >> 16) & 255, g = (px[i] >> 8) & 255, b = px[i] & 255;
            r = (r * a + 255 * (255 - a)) / 255;
            g = (g * a + 255 * (255 - a)) / 255;
            b = (b * a + 255 * (255 - a)) / 255;
            px[i] = unchecked((int)0xFF000000) | (r << 16) | (g << 8) | b;
            result[0]++;
            result[1] = Math.Min(result[1], i % w); result[2] = Math.Min(result[2], i / w);
            result[3] = Math.Max(result[3], i % w); result[4] = Math.Max(result[4], i / w);
        }
        Marshal.Copy(px, 0, data.Scan0, px.Length);
        bmp.UnlockBits(data);
        return result;
    }

    public static Bitmap Resize(Bitmap source, int size) {
        var result = new Bitmap(size, size, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(result))
        using (var wrap = new ImageAttributes()) {
            g.InterpolationMode = InterpolationMode.HighQualityBicubic;
            g.PixelOffsetMode = PixelOffsetMode.HighQuality;
            g.CompositingQuality = CompositingQuality.HighQuality;
            wrap.SetWrapMode(WrapMode.TileFlipXY);
            g.DrawImage(source, new Rectangle(0, 0, size, size), 0, 0, source.Width, source.Height, GraphicsUnit.Pixel, wrap);
        }
        return result;
    }

    // Classic icon image: 32-bit pixels stored bottom-up, then an (empty) 1-bit mask.
    public static byte[] Dib(Bitmap bmp) {
        int s = bmp.Width;
        BitmapData data;
        var px = Read(bmp, out data);
        bmp.UnlockBits(data);
        int maskRow = ((s + 31) / 32) * 4;
        var bytes = new byte[40 + s * s * 4 + maskRow * s];
        BitConverter.GetBytes(40).CopyTo(bytes, 0);
        BitConverter.GetBytes(s).CopyTo(bytes, 4);
        BitConverter.GetBytes(s * 2).CopyTo(bytes, 8);
        BitConverter.GetBytes((short)1).CopyTo(bytes, 12);
        BitConverter.GetBytes((short)32).CopyTo(bytes, 14);
        for (int y = 0; y < s; y++)
            Buffer.BlockCopy(px, (s - 1 - y) * s * 4, bytes, 40 + y * s * 4, s * 4);
        return bytes;
    }
}
"@

$art = New-Object System.Drawing.Bitmap $Source
$holes = [IconArt]::FillHoles($art)
if ($holes[0] -gt 0) {
    "Filled a see-through hole of $($holes[0]) pixels at x $($holes[1])..$($holes[3]), y $($holes[2])..$($holes[4])"
} else {
    "No see-through holes in the artwork"
}

$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
$images = foreach ($size in $sizes) {
    $bmp = [IconArt]::Resize($art, $size)
    if ($size -ge 256) {
        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $bytes = $ms.ToArray()
    } else {
        $bytes = [IconArt]::Dib($bmp)
    }
    $bmp.Dispose()
    [pscustomobject]@{ Size = $size; Bytes = $bytes }
}
$art.Dispose()

$out = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $out
$w.Write([int16]0); $w.Write([int16]1); $w.Write([int16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($i in $images) {
    $w.Write([byte]($i.Size % 256)); $w.Write([byte]($i.Size % 256)); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([int16]1); $w.Write([int16]32); $w.Write([int]$i.Bytes.Length); $w.Write([int]$offset)
    $offset += $i.Bytes.Length
}
foreach ($i in $images) { $w.Write([byte[]]$i.Bytes) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $PSScriptRoot 'icon.ico'), $out.ToArray())
"Wrote icon.ico ($($out.Length) bytes, sizes: $($sizes -join ', ')) from $(Split-Path $Source -Leaf)"
