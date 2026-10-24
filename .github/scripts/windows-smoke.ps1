# Installs the Windows beta the way a user without administrator rights
# would, runs it, and keeps screenshots and the log in $OutputDir:
#   desktop-first-launch.png  the desktop at the first launch (onboarding)
#   settings-window.png       the settings window once onboarding is done
#   desktop-overlay.png       the desktop while a dictation is toggled on
#   murmur.log                the app log
# Fails when the install is not per-user, lacks the Visual C++ runtime beside
# the app, the app loads that runtime from elsewhere, quits, the settings
# window goes past the work area (under the taskbar), or the log shows a panic.
param(
    [Parameter(Mandatory)] [string] $Installer,
    [Parameter(Mandatory)] [string] $OutputDir
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Native {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
}
'@
# Screen coordinates in physical pixels, as the screenshots are.
[Native]::SetProcessDPIAware() | Out-Null

function Save-Region([string] $Name, [System.Drawing.Rectangle] $Bounds) {
    $bitmap = New-Object System.Drawing.Bitmap $Bounds.Width, $Bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($Bounds.Location, [System.Drawing.Point]::Empty, $Bounds.Size)
    $bitmap.Save((Join-Path $OutputDir $Name), [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
    Write-Host "Saved $Name ($($Bounds.Width)x$($Bounds.Height))"
}

function Save-Desktop([string] $Name) {
    Save-Region $Name ([System.Windows.Forms.Screen]::PrimaryScreen.Bounds)
}

function Save-Window([System.Diagnostics.Process] $Process, [string] $Name) {
    $Process.Refresh()
    $window = $Process.MainWindowHandle
    if ($window -eq [IntPtr]::Zero) { throw 'Murmur shows no window' }
    [Native]::SetForegroundWindow($window) | Out-Null
    Start-Sleep -Seconds 2
    $rect = New-Object Native+RECT
    [Native]::GetWindowRect($window, [ref] $rect) | Out-Null
    Save-Region $Name ([System.Drawing.Rectangle]::FromLTRB($rect.Left, $rect.Top, $rect.Right, $rect.Bottom))
}

function Assert-InsideWorkArea([System.Diagnostics.Process] $Process) {
    $window = $Process.MainWindowHandle
    $rect = New-Object Native+RECT
    [Native]::GetWindowRect($window, [ref] $rect) | Out-Null
    $area = [System.Windows.Forms.Screen]::FromHandle($window).WorkingArea
    Write-Host "Window ($($rect.Left), $($rect.Top), $($rect.Right), $($rect.Bottom)) in work area $area"
    if ($rect.Left -lt $area.Left -or $rect.Top -lt $area.Top -or
        $rect.Right -gt $area.Right -or $rect.Bottom -gt $area.Bottom) {
        throw 'The settings window goes past the work area'
    }
}

$logFile = Join-Path $env:LOCALAPPDATA 'com.dailyxplorer.murmur\logs\murmur.log'
$settingsFile = Join-Path $env:APPDATA 'com.dailyxplorer.murmur\settings_store.json'
$installDir = Join-Path $env:LOCALAPPDATA 'Murmur'

function Start-Murmur {
    $process = Start-Process -FilePath $script:exe -PassThru
    Start-Sleep -Seconds 20
    $process.Refresh()
    if ($process.HasExited) { throw "Murmur quit at launch with code $($process.ExitCode)" }
    $process
}

function Stop-Murmur([System.Diagnostics.Process] $Process) {
    Stop-Process -Id $Process.Id -Force
    $Process.WaitForExit(10000) | Out-Null
    Start-Sleep -Seconds 2
}

function Assert-NoPanic {
    if (-not (Test-Path $logFile)) { throw "No log at $logFile" }
    if (Select-String -Path $logFile -Pattern 'panicked' -Quiet) {
        Select-String -Path $logFile -Pattern 'panicked' -Context 5, 5 | Out-String | Write-Host
        throw 'The log shows a panic'
    }
}

try {
    # A per-user NSIS installer runs silently without elevation, installs
    # under %LOCALAPPDATA% and registers its uninstaller under HKCU.
    $setup = Start-Process -FilePath $Installer -ArgumentList '/S' -Wait -PassThru
    if ($setup.ExitCode -ne 0) { throw "The installer exited with code $($setup.ExitCode)" }
    $script:exe = Get-ChildItem -Path $installDir -Filter '*.exe' |
        Where-Object Name -NotLike 'uninstall*' |
        Select-Object -First 1 -ExpandProperty FullName
    if (-not $script:exe) { throw "No Murmur executable under $installDir" }
    Write-Host "Installed $script:exe"
    if (-not (Test-Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Murmur')) {
        throw 'The installer registered no per-user uninstaller'
    }
    # Local transcription: transcribe.cpp and one CPU module per instruction
    # set, beside the app.
    $dlls = Get-ChildItem -Path $installDir -Filter '*.dll' | Select-Object -ExpandProperty Name
    Write-Host "Installed DLLs: $($dlls -join ', ')"
    if ($dlls -notcontains 'transcribe.dll') { throw 'The installer shipped no transcribe.dll' }
    if (-not ($dlls | Where-Object { $_ -like 'ggml-cpu*' })) { throw 'The installer shipped no CPU module' }
    # The Visual C++ runtime, for a PC without the redistributable.
    foreach ($required in 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll', 'vcomp140.dll') {
        if ($dlls -notcontains $required) { throw "The installer shipped no $required" }
    }

    $murmur = Start-Murmur
    # The runner has the redistributable in System32: the app must load the
    # runtime from its own folder all the same.
    $murmur.Refresh()
    $runtime = $murmur.Modules | Where-Object { $_.ModuleName -match '^(msvcp140|vcruntime140|vcomp140)' }
    $runtime | ForEach-Object { Write-Host "Loaded $($_.FileName)" }
    if (-not ($runtime | Where-Object ModuleName -eq 'vcruntime140.dll')) { throw 'Murmur loaded no vcruntime140.dll' }
    $outside = $runtime | Where-Object { (Split-Path $_.FileName) -ne $installDir }
    if ($outside) { throw "Murmur loaded the Visual C++ runtime from outside its folder: $($outside.FileName -join ', ')" }
    Save-Desktop 'desktop-first-launch.png'
    Stop-Murmur $murmur

    # As after onboarding: the settings window opens instead.
    $settings = Get-Content -Raw $settingsFile | ConvertFrom-Json
    $settings.settings.onboarding_completed = $true
    $settings | ConvertTo-Json -Depth 32 | Set-Content -Encoding utf8NoBOM $settingsFile

    $murmur = Start-Murmur
    Save-Window $murmur 'settings-window.png'
    Assert-InsideWorkArea $murmur

    # A second instance forwards the toggle to the running one, then quits.
    # Without a microphone on the runner, the overlay reports it.
    Start-Process -FilePath $script:exe -ArgumentList '--toggle-transcription' -Wait
    Start-Sleep -Seconds 2
    Save-Desktop 'desktop-overlay.png'
    Start-Process -FilePath $script:exe -ArgumentList '--cancel' -Wait
    Start-Sleep -Seconds 3

    $murmur.Refresh()
    if ($murmur.HasExited) { throw "Murmur quit with code $($murmur.ExitCode)" }
    Assert-NoPanic
    Stop-Murmur $murmur

    $uninstall = Start-Process -FilePath (Join-Path $installDir 'uninstall.exe') -ArgumentList '/S' -Wait -PassThru
    if ($uninstall.ExitCode -ne 0) { throw "The uninstaller exited with code $($uninstall.ExitCode)" }
    Write-Host 'Smoke test passed'
}
finally {
    if (Test-Path $logFile) { Copy-Item $logFile (Join-Path $OutputDir 'murmur.log') }
}
