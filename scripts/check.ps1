# button full check suite (spec 11, D-059).
# Runs on Windows PowerShell 5.1 and GitHub Actions windows-latest.
# ASCII only: PowerShell 5.1 reads scripts without a BOM as ANSI.

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot

$failed = $false

function Run-Step {
    param([string]$Name)
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL  $Name"
        $script:failed = $true
    } else {
        Write-Host "ok    $Name"
    }
}

Write-Host "== button: check.ps1 =="

cargo fmt --check --manifest-path "$repoRoot\app\src-tauri\Cargo.toml"
Run-Step "cargo fmt --check"

cargo clippy --manifest-path "$repoRoot\app\src-tauri\Cargo.toml" --all-targets -- -D warnings
Run-Step "cargo clippy -D warnings"

cargo test --manifest-path "$repoRoot\app\src-tauri\Cargo.toml"
Run-Step "cargo test"

Push-Location "$repoRoot\app"
npx tsc --noEmit
Run-Step "tsc --noEmit"

npx vitest run
Run-Step "vitest run"
Pop-Location

Write-Host "== done =="
if ($failed) { exit 1 } else { exit 0 }
