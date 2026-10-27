$ErrorActionPreference = 'Stop'
$repo = 'ManiProjs/alacoup'
$version = if ($env:ALACOUP_VERSION) { $env:ALACOUP_VERSION } else { 'latest' }
$root = if ($env:ALACOUP_HOME) { $env:ALACOUP_HOME } else { Join-Path $HOME '.alacoup' }
$binDir = Join-Path $root 'bin'
$archiveName = 'alacoup-x86_64-pc-windows-msvc.zip'
if ($version -eq 'latest') {
  $url = "https://github.com/$repo/releases/latest/download/$archiveName"
} else {
  if ($version -notmatch '^v\d+\.\d+\.\d+([+-][0-9A-Za-z.-]+)?$') { throw 'ALACOUP_VERSION must be a release tag such as v0.1.0' }
  $url = "https://github.com/$repo/releases/download/$version/$archiveName"
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
$stage = Join-Path $tmp 'stage'
try {
  New-Item -ItemType Directory -Force $stage | Out-Null
  Invoke-WebRequest -Uri $url -OutFile (Join-Path $tmp 'alacoup.zip')
  Expand-Archive -Path (Join-Path $tmp 'alacoup.zip') -DestinationPath $stage
  $source = Join-Path $stage 'alacoup.exe'
  if (!(Test-Path $source -PathType Leaf)) { throw 'Release archive did not contain alacoup.exe' }
  New-Item -ItemType Directory -Force $binDir | Out-Null
  $stagedExe = Join-Path $binDir 'alacoup.exe.new'
  Copy-Item $source $stagedExe -Force
  Move-Item $stagedExe (Join-Path $binDir 'alacoup.exe') -Force
} finally {
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host "Installed AlacoUp into $binDir"
& (Join-Path $binDir 'alacoup.exe') install
if ($LASTEXITCODE -ne 0) { throw 'AlacoUp was installed, but Alaco installation failed. Run alacoup.exe install to retry.' }
Write-Host 'AlacoUp and Alaco installation finished. Run: alacoup.exe doctor'
