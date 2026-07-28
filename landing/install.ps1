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

# --- 0 - Is there room? ------------------------------------------------------
# Asked BEFORE the 2 GB download: someone whose machine cannot run the model
# should learn it before spending the bandwidth. On Linux the same check exists
# because a short machine had the first chat killed outright by the kernel;
# Windows swaps instead of killing, which is slower but not clearer.
$needMb = 4096
$tight = $false
try {
    $freeMb = [int]((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1024)
    if ($freeMb -lt $needMb) {
        $tight = $true
        Write-Warning "About $freeMb MB of memory is free and the model wants ~$needMb MB."
        Write-Warning "The install will finish, but the first chat is not started - free some"
        Write-Warning "memory (or use a smaller model) and run:  tacet chat"
    }
} catch {
    # Not knowing is fine; guessing is not. The install continues.
}

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

# --- 4 - Can the user actually type `tacet`? ---------------------------------
# THIS SCRIPT USED TO LIE HERE. `$env:Path` is set for THIS process; the window
# the reader is sitting in never sees it, so "tacet chat" answered with a
# command-not-found. rustup writes the machine PATH entry, but an already-open
# terminal keeps the environment it started with.
$onPath = $null -ne (Get-Command tacet -ErrorAction SilentlyContinue)

Write-Host ""
Write-Host "done."
if (-not $onPath) {
    Write-Host ""
    Write-Host "one more step - tacet is installed but not on this window's PATH."
    Write-Host "Open a NEW terminal, or for this one:"
    Write-Host ""
    Write-Host '  $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"'
    Write-Host ""
    Write-Host "Installed at: $tacet"
}

# --- 5 - First chat ----------------------------------------------------------
if ($tight) {
    Write-Host ""
    Write-Host "not starting the chat: this machine is short on memory (see the warning above)."
    Write-Host "when there is room:  tacet chat"
} else {
    Write-Host ""
    Write-Host "starting tacet - type away, /quit leaves."
    Write-Host ""
    & $tacet chat
}
