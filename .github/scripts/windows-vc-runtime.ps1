# Finds the x64 Visual C++ runtime of the Visual Studio that compiles Murmur
# and sets MURMUR_VC_REDIST_DIRS for the next steps, so src-tauri/build.rs
# ships the runtime DLLs beside murmur.exe: the C++ runtime, and OpenMP,
# which transcribe.cpp's CPU code uses on Windows. Located as Handy does.

$ErrorActionPreference = 'Stop'
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsRoot = & $vswhere -latest -products * -property installationPath
if (-not $vsRoot) { throw 'vswhere found no Visual Studio' }
$version = (Get-Content (Join-Path $vsRoot 'VC\Auxiliary\Build\Microsoft.VCRedistVersion.default.txt')).Trim()
$archDir = Join-Path $vsRoot "VC\Redist\MSVC\$version\x64"
if (-not (Test-Path $archDir)) { throw "No Visual C++ redist at $archDir" }

# The folder carries the toolset generation (Microsoft.VC143.CRT); the DLL
# names do not change with it.
$dirs = foreach ($family in @{ CRT = 'msvcp140.dll'; OpenMP = 'vcomp140.dll' }.GetEnumerator()) {
    $dir = Get-ChildItem $archDir -Directory -Filter "Microsoft.VC*.$($family.Key)" |
        Sort-Object Name | Select-Object -Last 1
    if (-not $dir -or -not (Test-Path (Join-Path $dir.FullName $family.Value))) {
        throw "No $($family.Value) under $archDir"
    }
    $dir.FullName
}
$value = $dirs -join ';'
Write-Host "MURMUR_VC_REDIST_DIRS=$value"
Add-Content -Path $env:GITHUB_ENV -Value "MURMUR_VC_REDIST_DIRS=$value"
