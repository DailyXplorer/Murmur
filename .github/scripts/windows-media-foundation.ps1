# The meeting audio archive tests encode AAC with Media Foundation. Windows
# Server images ship without it (the Server-Media-Foundation feature); Home
# and Pro editions have it. Installs the feature when a library is missing,
# then fails if it is still not there.
$ErrorActionPreference = 'Stop'

$libraries = 'mfplat.dll', 'mfreadwrite.dll', 'mfaacenc.dll'
$system = Join-Path $env:SystemRoot 'System32'

function Get-Missing {
    $libraries | Where-Object { -not (Test-Path (Join-Path $system $_)) }
}

$missing = @(Get-Missing)
if ($missing.Count -eq 0) {
    Write-Host 'Media Foundation and its AAC encoder are installed.'
    exit 0
}

Write-Host "Missing: $($missing -join ', '). Installing Server-Media-Foundation."
Install-WindowsFeature -Name Server-Media-Foundation | Out-Host

$missing = @(Get-Missing)
if ($missing.Count -gt 0) {
    throw "Still missing after the install: $($missing -join ', ')"
}
Write-Host 'Media Foundation and its AAC encoder are installed.'
