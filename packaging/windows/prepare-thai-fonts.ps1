# Prepare the optional font checkout for a reproducible Thai editing update.
# Font binaries stay outside PdfCraft, in craft-fonts (AGENTS.md section 1.4).
param([Parameter(Mandatory = $true)][string] $FontRoot)
$ErrorActionPreference = 'Stop'
$FontRoot = [IO.Path]::GetFullPath($FontRoot)
$CraftRef = '3d9ab3a132ed0eff457e424226a435a901da30d3'
$SourceRef = '88058c714073e72bb1b32baffc2d4ce9bc73944c'
if (-not (Test-Path -LiteralPath $FontRoot)) {
  git clone https://github.com/storytold/craft-fonts.git $FontRoot
  if ($LASTEXITCODE -ne 0) { throw 'could not clone craft-fonts' }
}
$ActualRef = git -C $FontRoot rev-parse HEAD
if ($LASTEXITCODE -ne 0 -or $ActualRef -ne $CraftRef) { throw "craft-fonts must be at $CraftRef" }
$Dir = Join-Path $FontRoot 'fonts/sarabun'
New-Item -ItemType Directory -Force -Path $Dir | Out-Null
$Expected = [ordered]@{
  'Sarabun-Regular.ttf' = '226d4f368fbc0457990ddef2692679badfd2c1a4e89e5ac4d43c10ba7743b2f1'
  'Sarabun-Bold.ttf' = 'd38308ca27d067b9a1b79006337c1f9a66c29aa016d96a9d88d3801da7fa83b9'
  'OFL.txt' = 'b26cae1321380296ba8311b632a397d5eac11b47197f9d0aa0b9310f1531ad60'
}
$Source = "https://github.com/google/fonts/tree/$SourceRef/ofl/sarabun"
foreach ($File in $Expected.Keys) {
  $Path = Join-Path $Dir $File
  if (-not (Test-Path -LiteralPath $Path)) {
    Invoke-WebRequest "https://raw.githubusercontent.com/google/fonts/$SourceRef/ofl/sarabun/$File" -OutFile $Path
  }
  if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Expected[$File]) {
    throw "SHA-256 mismatch: $Path"
  }
}
$Utf8 = New-Object Text.UTF8Encoding($false)
$ManifestPath = Join-Path $FontRoot 'fonts/manifest.txt'
$AttributionPath = Join-Path $FontRoot 'ATTRIBUTION.md'
$Manifest = [IO.File]::ReadAllText($ManifestPath)
$Attribution = [IO.File]::ReadAllText($AttributionPath)
foreach ($Weight in @('Regular', 'Bold')) {
  $File = "Sarabun-$Weight.ttf"
  $Relative = "fonts/sarabun/$File"
  if (-not $Manifest.Contains($Relative)) {
    $Manifest += "`nSarabun | $Weight | $Relative | Thai,Latn | OFL-1.1 | fonts/sarabun/OFL.txt | $($Expected[$File]) | $Source`n"
  }
  if (-not $Attribution.Contains($Relative)) {
    $Attribution += "`n| $Relative | Sarabun $Weight | The Sarabun Project Authors (cadsondemak) | $Source | OFL-1.1 (fonts/sarabun/OFL.txt), SHA-256 $($Expected[$File]) |`n"
  }
}
if (-not $Attribution.Contains($Expected['OFL.txt'])) {
  $Attribution += "`nSarabun licence: fonts/sarabun/OFL.txt; source $Source/OFL.txt; SHA-256 $($Expected['OFL.txt']).`n"
}
[IO.File]::WriteAllText($ManifestPath, $Manifest, $Utf8)
[IO.File]::WriteAllText($AttributionPath, $Attribution, $Utf8)
$env:CRAFT_FONTS_DIR = $FontRoot
$env:CRAFT_FONTS_REQUIRED = '1'
Write-Output "Thai editing fonts verified in $FontRoot"
