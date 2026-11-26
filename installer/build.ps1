$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
$env:CARGO_TARGET_DIR = Join-Path $root "src-tauri\target"

npm run tauri build
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$exe = Join-Path $env:CARGO_TARGET_DIR "release\biblioteka.exe"
if (-not (Test-Path $exe)) {
  throw "Nie znaleziono pliku aplikacji: $exe"
}

$iscc = Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"
& $iscc "/DExePath=$exe" (Join-Path $PSScriptRoot "biblioteka.iss")
exit $LASTEXITCODE
