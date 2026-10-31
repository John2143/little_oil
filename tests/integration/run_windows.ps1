# Windows integration test for `little_oil empty` against a fake game window.
#
# Starts `examples/fake_poe.exe` (a "Path of Exile" window with a synthetic
# 12x5 inventory at a fixed offset of its client area), points little_oil at
# that panel, then drives the real binary:
#
#   1. `little_oil.exe reset_inv` samples the empty inventory,
#   2. five items appear in the fake window,
#   3. `little_oil.exe empty` clicks every occupied cell,
#   4. the panel is captured with GDI and must show an empty inventory.
#
# The fake reports every pointer press on stdout (`CLICK left <x> <y> ctrl=<0|1>
# cell=<col>,<row>`), so the test can assert each item cell was clicked exactly
# once with Ctrl held — Ctrl+click is what moves the item out of the inventory.
#
# Exit codes:
#   0  pass
#   1  assertion failure (a real bug)
#   77 environment limit — a hosted runner cannot run this: the session may be
#      locked (SendInput is dropped), OpenGL 3.3 may be missing so the fake
#      window never appears, or WGC cannot create a D3D11 hardware device so
#      `little_oil`'s capture fails. The CI step turns 77 into a warning.
#
# Environment:
#   LITTLE_OIL_BIN  little_oil.exe (default target\release\little_oil.exe)
#   FAKE_POE_BIN    fake_poe.exe   (default target\release\examples\fake_poe.exe)
# Both are built with `cargo build --release --bin little_oil --example fake_poe`
# when missing.

$ErrorActionPreference = 'Stop'

# The fake paints these flat colors (see examples/fake_poe.rs).
$EMPTY_RGB = @(40, 40, 40)
$ITEM_RGB = @(200, 120, 40)
$TOLERANCE = 16
# Inventory panel: offset inside the client area and its size, in logical points.
$PANEL_X = 40
$PANEL_Y = 40
$PANEL_W = 1200
$PANEL_H = 600
# The window asks for this inner size; the client area / this tells the DPI scale.
$WINDOW_W = 1280.0

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$littleOil = if ($env:LITTLE_OIL_BIN) { $env:LITTLE_OIL_BIN } else { Join-Path $root 'target\release\little_oil.exe' }
$fakePoe = if ($env:FAKE_POE_BIN) { $env:FAKE_POE_BIN } else { Join-Path $root 'target\release\examples\fake_poe.exe' }
$configPath = Join-Path $env:APPDATA 'little_oil\config.json'

function Skip-Environment([string]$reason) {
    Write-Host "SKIP: $reason"
    exit 77
}

function Fail([string]$message) {
    Write-Host "FAIL: $message"
    exit 1
}

# Pure user32/gdi32 P/Invoke: System.Drawing on PowerShell 7 / .NET 8 needs
# extra assembly references (`IImage` lives in a private assembly).
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class WinProbe {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X, Y; }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string windowName);

    [DllImport("user32.dll")]
    public static extern bool GetClientRect(IntPtr hwnd, out RECT rect);

    [DllImport("user32.dll")]
    public static extern bool ClientToScreen(IntPtr hwnd, ref POINT point);

    [DllImport("user32.dll")]
    public static extern IntPtr SetProcessDpiAwarenessContext(IntPtr context);

    [StructLayout(LayoutKind.Sequential)]
    struct BITMAPINFOHEADER {
        public int biSize, biWidth, biHeight;
        public short biPlanes, biBitCount;
        public int biCompression, biSizeImage, biXPelsPerMeter, biYPelsPerMeter, biClrUsed, biClrImportant;
    }

    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleBitmap(IntPtr dc, int w, int h);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr dc, IntPtr obj);
    [DllImport("gdi32.dll")] static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, int rop);
    [DllImport("gdi32.dll")] static extern int GetDIBits(IntPtr dc, IntPtr bmp, uint start, uint lines, byte[] bits, ref BITMAPINFOHEADER bmi, uint usage);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);

    // Count pixels of the screen region (x, y, w, h) whose RGB is within
    // `tolerance` of (r, g, b). GDI BitBlt reads the DWM-composited desktop.
    public static int CountNear(int x, int y, int w, int h, int r, int g, int b, int tolerance) {
        IntPtr screen = GetDC(IntPtr.Zero);
        IntPtr mem = CreateCompatibleDC(screen);
        IntPtr bmp = CreateCompatibleBitmap(screen, w, h);
        IntPtr old = SelectObject(mem, bmp);
        try {
            const int SRCCOPY = 0x00CC0020;
            if (!BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY)) throw new Exception("BitBlt failed");
            SelectObject(mem, old);
            var bmi = new BITMAPINFOHEADER {
                biSize = Marshal.SizeOf(typeof(BITMAPINFOHEADER)),
                biWidth = w, biHeight = -h, biPlanes = 1, biBitCount = 32,
            };
            var bytes = new byte[w * h * 4];
            if (GetDIBits(mem, bmp, 0, (uint)h, bytes, ref bmi, 0) == 0) throw new Exception("GetDIBits failed");
            int count = 0;
            for (int i = 0; i < bytes.Length; i += 4) {
                if (Math.Abs(bytes[i + 2] - r) <= tolerance &&
                    Math.Abs(bytes[i + 1] - g) <= tolerance &&
                    Math.Abs(bytes[i] - b) <= tolerance) {
                    count++;
                }
            }
            return count;
        } finally {
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(IntPtr.Zero, screen);
        }
    }
}
'@

# Physical pixels in GetClientRect/CopyFromScreen, matching little_oil's
# per-monitor DPI awareness.
[void][WinProbe]::SetProcessDpiAwarenessContext([IntPtr](-4))

foreach ($tool in @($littleOil, $fakePoe)) {
    if (-not (Test-Path $tool)) {
        Write-Host "== building little_oil and fake_poe"
        Push-Location $root
        try {
            & cargo build --release --bin little_oil --example fake_poe
            if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
        } finally {
            Pop-Location
        }
        break
    }
}
if (-not (Test-Path $littleOil)) { Fail "little_oil binary not found: $littleOil" }
if (-not (Test-Path $fakePoe)) { Fail "fake_poe binary not found: $fakePoe" }

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("little_oil_windows_" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$itemsFile = Join-Path $work 'items'
Set-Content -Path $itemsFile -Value '0' -NoNewline
$fake = $null

# Back up the user's config so a local run does not clobber it.
$configBackup = $null
New-Item -ItemType Directory -Path (Split-Path -Parent $configPath) -Force | Out-Null
if (Test-Path $configPath) {
    $configBackup = "$configPath.bak"
    Move-Item -Path $configPath -Destination $configBackup -Force
}

try {
    Write-Host "== starting fake_poe"
    $env:FAKE_POE_ITEMS = $itemsFile
    $fake = Start-Process -FilePath $fakePoe -PassThru -WorkingDirectory $work `
        -RedirectStandardOutput (Join-Path $work 'fake.out') `
        -RedirectStandardError (Join-Path $work 'fake.err')

    $fakeOut = Join-Path $work 'fake.out'
    $ready = $false
    for ($i = 0; $i -lt 300; $i++) {
        if ((Test-Path $fakeOut) -and (Select-String -Path $fakeOut -Pattern '^READY$' -Quiet -ErrorAction SilentlyContinue)) {
            $ready = $true
            break
        }
        if ($fake.HasExited) { break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $ready) {
        $stderr = if (Test-Path (Join-Path $work 'fake.err')) { Get-Content (Join-Path $work 'fake.err') -Raw } else { '' }
        Skip-Environment "the fake window never opened (no OpenGL 3.3 on this runner?): $stderr"
    }

    $hwnd = [IntPtr]::Zero
    for ($i = 0; $i -lt 100; $i++) {
        $hwnd = [WinProbe]::FindWindow($null, 'Path of Exile')
        if ($hwnd -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($hwnd -eq [IntPtr]::Zero) { Skip-Environment 'the fake window is not on this desktop' }

    $rect = [WinProbe+RECT]::new()
    if (-not [WinProbe]::GetClientRect($hwnd, [ref]$rect)) { Skip-Environment 'GetClientRect failed' }
    $clientW = $rect.Right - $rect.Left
    $clientH = $rect.Bottom - $rect.Top
    $origin = [WinProbe+POINT]::new()
    $origin.X = 0
    $origin.Y = 0
    if (-not [WinProbe]::ClientToScreen($hwnd, [ref]$origin)) { Skip-Environment 'ClientToScreen failed' }

    # DPI scale: the window was created 1280 points wide, so its physical client
    # width is the scale factor. The fake draws in points, so the panel offset
    # and size scale with it.
    $scale = $clientW / $WINDOW_W
    if ($scale -lt 0.5 -or $scale -gt 4.0) { Fail "implausible DPI scale $scale (client ${clientW}x${clientH})" }
    $invX = $origin.X + [int][Math]::Round($PANEL_X * $scale)
    $invY = $origin.Y + [int][Math]::Round($PANEL_Y * $scale)
    $invW = [int][Math]::Round($PANEL_W * $scale)
    $invH = [int][Math]::Round($PANEL_H * $scale)
    Write-Host "== fake client area ${clientW}x${clientH} at ($($origin.X), $($origin.Y)); panel ${invW}x${invH} at ($invX, $invY); scale $scale"

    $config = [ordered]@{
        focus_clicks       = 0
        pointer_scale      = 1.0
        inv_region         = [ordered]@{ x = $invX; y = $invY; width = $invW; height = $invH }
        game_window_region = [ordered]@{ x = $origin.X; y = $origin.Y; width = $clientW; height = $clientH }
    }
    $config | ConvertTo-Json -Depth 5 | Set-Content -Path $configPath -Encoding utf8

    function Count-ItemPixels {
        return [WinProbe]::CountNear($invX, $invY, $invW, $invH,
            $ITEM_RGB[0], $ITEM_RGB[1], $ITEM_RGB[2], $TOLERANCE)
    }

    Write-Host '== little_oil reset_inv (empty inventory)'
    & $littleOil reset_inv 2>&1 | Tee-Object -FilePath (Join-Path $work 'little_oil.log')
    if ($LASTEXITCODE -ne 0) {
        $log = Get-Content (Join-Path $work 'little_oil.log') -Raw
        if ($log -match 'D3D11CreateDevice|WGC|GraphicsCapture') {
            Skip-Environment "little_oil cannot capture on this runner: $log"
        }
        Fail "reset_inv exited $LASTEXITCODE"
    }

    $samples = (Get-Content $configPath -Raw | ConvertFrom-Json).inv_samples
    if ($null -eq $samples -or $samples.Count -ne 60) {
        Fail "reset_inv should save 60 slots x 3 samples, got $($samples.Count)"
    }
    Write-Host '   saved 60 slots x 3 samples of the empty inventory'

    Write-Host '== placing 5 items (diagonal pattern: both axes, non-contiguous)'
    Set-Content -Path $itemsFile -Value 'diagonal' -NoNewline
    $before = 0
    for ($i = 0; $i -lt 100; $i++) {
        $before = Count-ItemPixels
        if ($before -gt 1000) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($before -le 1000) {
        Skip-Environment "the fake inventory never showed items on screen (black capture? $before item pixels)"
    }
    Write-Host "   $before item-colored pixels on screen"

    Write-Host '== little_oil empty'
    & $littleOil empty 2>&1 | Tee-Object -FilePath (Join-Path $work 'little_oil.log') -Append
    if ($LASTEXITCODE -ne 0) {
        $log = Get-Content (Join-Path $work 'little_oil.log') -Raw
        if ($log -match 'D3D11CreateDevice|WGC|GraphicsCapture|timed out waiting for WGC frame') {
            Skip-Environment "little_oil cannot capture on this runner: $log"
        }
        Fail "empty exited $LASTEXITCODE"
    }

    $after = -1
    for ($i = 0; $i -lt 100; $i++) {
        $after = Count-ItemPixels
        if ($after -eq 0) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($after -ne 0) { Fail "inventory is not empty after empty: $after item pixels left" }

    $lines = Get-Content $fakeOut
    $leftClicks = @($lines | Where-Object { $_ -match '^CLICK left' })
    $allClicks = @($lines | Where-Object { $_ -match '^CLICK ' })
    if ($leftClicks.Count -ne 5) { Fail "expected 5 left clicks, got $($leftClicks.Count):`n$($allClicks -join "`n")" }
    if ($allClicks.Count -ne 5) { Fail "non-left clicks were sent:`n$($allClicks -join "`n")" }
    if ($leftClicks | Where-Object { $_ -notmatch 'ctrl=1' }) {
        Fail "a click arrived without Ctrl held:`n$($allClicks -join "`n")"
    }
    if ($leftClicks | Where-Object { $_ -notmatch 'cell=\d+,\d+$' }) {
        Fail "a click landed outside the inventory panel:`n$($allClicks -join "`n")"
    }
    $cells = @($leftClicks | ForEach-Object { ($_ -split 'cell=')[1] } | Sort-Object)
    $expected = @('0,0', '3,1', '6,2', '9,3', '11,4') | Sort-Object
    if (Compare-Object $cells $expected) {
        Fail "clicks did not cover each item cell exactly once: $($cells -join ' ')"
    }

    Write-Host 'PASS: 5 occupied cells clicked once each with Ctrl held; inventory is empty'
    exit 0
} catch {
    Write-Host "FAIL: $($_.Exception.Message)"
    foreach ($name in @('fake.out', 'fake.err', 'little_oil.log')) {
        $path = Join-Path $work $name
        if (Test-Path $path) {
            Write-Host "--- $name (tail)"
            Get-Content $path -Tail 30 | Write-Host
        }
    }
    exit 1
} finally {
    if ($null -ne $fake -and -not $fake.HasExited) {
        Stop-Process -Id $fake.Id -Force -ErrorAction SilentlyContinue
    }
    Remove-Item -Path $configPath -Force -ErrorAction SilentlyContinue
    if ($null -ne $configBackup -and (Test-Path $configBackup)) {
        Move-Item -Path $configBackup -Destination $configPath -Force
    }
    Remove-Item -Path $work -Recurse -Force -ErrorAction SilentlyContinue
}
