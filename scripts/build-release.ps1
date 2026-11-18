# Builds everything a release needs into dist\:
#
#   SparkyPDF-arm64.exe   portable program for Windows on ARM
#   SparkyPDF-x64.exe     portable program for ordinary 64-bit PCs
#   SparkyPDF-Setup.exe   installer for both (only when Inno Setup 6.3+ is installed)
#   SHA256SUMS.txt        checksums of the files above
#
#   powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1
param([switch]$SkipInstaller)

$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
Get-ChildItem $dist -File | Remove-Item -Force

$targets = [ordered]@{ arm64 = 'aarch64-pc-windows-msvc'; x64 = 'x86_64-pc-windows-msvc' }
foreach ($arch in $targets.Keys) {
    $target = $targets[$arch]
    Write-Host "== Building $arch ($target)"
    rustup target add $target
    cargo build --release --target $target
    if ($LASTEXITCODE -ne 0) { Write-Host "Build failed for $arch"; exit 1 }
    Copy-Item "target\$target\release\SparkyPDF.exe" (Join-Path $dist "SparkyPDF-$arch.exe")
}

$iscc = Get-ChildItem -ErrorAction SilentlyContinue -Path @(
    "${env:ProgramFiles(x86)}\Inno Setup *\ISCC.exe",
    "$env:ProgramFiles\Inno Setup *\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup *\ISCC.exe"
) | Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
if (-not $iscc) { $iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source }

if ($SkipInstaller) {
    Write-Host "== Skipping the installer"
} elseif ($iscc) {
    Write-Host "== Building the installer"
    & $iscc /Qp "/DAppVersion=$version" "/O$dist" installer\SparkyPDF.iss
    if ($LASTEXITCODE -ne 0) { Write-Host "Installer build failed"; exit 1 }
} else {
    Write-Host "== Inno Setup is not installed: no installer was built"
}

$sums = Get-ChildItem $dist -Filter *.exe | Sort-Object Name | ForEach-Object {
    "$((Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower())  $($_.Name)"
}
# Unix line endings, so that `sha256sum -c SHA256SUMS.txt` accepts the file.
[System.IO.File]::WriteAllText((Join-Path $dist 'SHA256SUMS.txt'), (($sums -join "`n") + "`n"))

Write-Host "== Sparky PDF $version"
Get-ChildItem $dist | ForEach-Object { Write-Host ("{0,10:N0} bytes  {1}" -f $_.Length, $_.Name) }
