$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$config = (Resolve-Path (Join-Path $PSScriptRoot '../../src-tauri/.config/nextest.toml')).Path
$directory = Join-Path ([System.IO.Path]::GetTempPath()) "murmur-timeout-$([Guid]::NewGuid())"
New-Item -ItemType Directory -Path $directory | Out-Null

try {
    [System.IO.File]::WriteAllText((Join-Path $directory 'Cargo.toml'), @'
[package]
name = "murmur-timeout-probe"
version = "0.0.0"
edition = "2021"
[lib]
path = "lib.rs"
'@)
    [System.IO.File]::WriteAllText((Join-Path $directory 'lib.rs'), @'
#[test]
fn timeout_probe_hangs() {
    loop {
        std::thread::park();
    }
}
'@)

    # Use the real CI profile, including its two-minute timeout. A build or
    # discovery failure must not pass for a test that nextest terminated.
    $output = & cargo nextest run --manifest-path (Join-Path $directory 'Cargo.toml') --config-file $config --profile ci --color never 2>&1
    $code = $LASTEXITCODE
    $text = $output | Out-String
    Write-Host $text
    if ($code -ne 100 -or $text -notmatch 'TIMEOUT[^\r\n]*timeout_probe_hangs') {
        throw "Expected nextest to time out timeout_probe_hangs with exit code 100, got $code"
    }
} finally {
    Remove-Item -LiteralPath $directory -Recurse -Force
}
