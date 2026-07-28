# Tacet installer — Windows (PowerShell). ONE paste does everything:
#   1. installs Rust's build tool (rustup) IF cargo is missing,
#   2. builds and installs the `tacet` binary with cargo,
#   3. downloads the model (one-time, ~2 GB, stays on your disk),
#   4. starts your first chat.
# Apart from the two downloads above nothing is sent anywhere: Tacet runs on
# this machine.
$ErrorActionPreference = "Stop"
$Model = "qwen3-4b"

Write-Host ""
Write-Host "Tacet."
Write-Host "the quiet assistant - installing on this machine"
Write-Host ""

# --- 1 - Rust toolchain ------------------------------------------------------
$cargoHome = Join-Path $env:USERPROFILE ".cargo\bin"
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    if (Test-Path (Join-Path $cargoHome "cargo.exe")) {
        $env:Path = "$cargoHome;$env:Path"
    } else {
        Write-Host "- rust is not installed - getting rustup (the official installer)"
        $rustupInit = Join-Path $env:TEMP "rustup-init.exe"
        Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupInit
        & $rustupInit -y --profile minimal
        $env:Path = "$cargoHome;$env:Path"
    }
}
Write-Host "- cargo found: $((Get-Command cargo).Source)"

# --- 2 - Tacet ---------------------------------------------------------------
Write-Host "- building tacet - a few minutes on first install"
cargo install tacet-cli --features candle

$tacet = Join-Path $cargoHome "tacet.exe"
if (-not (Test-Path $tacet)) { $tacet = "tacet" }

# --- 3 - The model -----------------------------------------------------------
# Skipped when the model folder already exists. `--no-approval` only skips
# tacet's own "download? [Y/n]" prompt - running this installer IS that answer,
# and the script announced the download at the top.
$modelDirs = @(
    (Join-Path $env:USERPROFILE "models\$Model"),
    (Join-Path $env:LOCALAPPDATA "Tacet\models\$Model")
)
if ($modelDirs | Where-Object { Test-Path $_ }) {
    Write-Host "- model $Model already on disk - skipping the download"
} else {
    Write-Host "- downloading $Model (one-time, ~2 GB, stays on your disk)"
    & $tacet models download $Model --no-approval
}

# --- 4 - First chat ----------------------------------------------------------
Write-Host ""
Write-Host "done. starting tacet - type away, /quit leaves."
Write-Host ""
& $tacet chat
