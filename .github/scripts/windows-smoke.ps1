# Installs the Windows beta the way a user without administrator rights
# would, runs it, and keeps screenshots and the log in $OutputDir:
#   desktop-first-launch.png  the desktop at the first launch (onboarding)
#   settings-window.png       the settings window once onboarding is done
#   desktop-overlay.png       the desktop while a dictation is toggled on
#   murmur.log                the app log
# Meetings are on by default there: the settings the first launch writes say
# so, the library `meetings.db` exists once the app runs, and the AI assistant
# server lists the (empty) library. Recording a meeting needs audio devices
# the runner lacks; the Rust tests run a whole meeting on fake capture.
# Fails when the install is not per-user, lacks the Visual C++ runtime beside
# the app, the app loads that runtime from elsewhere, quits, the settings
# window goes past the work area (under the taskbar), the log shows a panic, or
# `murmur.exe mcp`, run beside the app with redirected pipes as an AI assistant
# runs it, fails to answer, shows a window or outlives its stdin.
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
$meetingsDatabase = Join-Path $env:APPDATA 'com.dailyxplorer.murmur\meetings.db'
$installDir = Join-Path $env:LOCALAPPDATA 'Murmur'
$roamingData = Join-Path $env:APPDATA 'com.dailyxplorer.murmur'
$localData = Join-Path $env:LOCALAPPDATA 'com.dailyxplorer.murmur'

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

# The release exe has the GUI subsystem: no console, so no console window for
# an assistant that spawns `murmur mcp`. Its stdin and stdout are the pipes the
# parent passes in. A console-subsystem exe would pass this check too, so the
# PE header is read first.
function Assert-McpServer {
    $bytes = [System.IO.File]::ReadAllBytes($script:exe)
    $optionalHeader = [BitConverter]::ToInt32($bytes, 0x3C) + 24
    $subsystem = [BitConverter]::ToUInt16($bytes, $optionalHeader + 68)
    if ($subsystem -ne 2) { throw "The installed exe has PE subsystem $subsystem, not 2 (Windows GUI)" }

    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $script:exe
    $info.Arguments = 'mcp'
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardInput = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardInputEncoding = New-Object System.Text.UTF8Encoding $false
    $server = [System.Diagnostics.Process]::Start($info)
    try {
        $server.StandardInput.WriteLine('{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}')
        $server.StandardInput.WriteLine('{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}')
        $server.StandardInput.WriteLine('{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_meetings","arguments":{}}}')
        $server.StandardInput.Flush()
        $replies = foreach ($id in 1, 2, 3) {
            $line = $server.StandardOutput.ReadLineAsync()
            if (-not $line.Wait(20000)) { throw "murmur mcp sent no reply to request $id within 20 s" }
            $line.Result | ConvertFrom-Json
        }
        $server.Refresh()
        if ($server.MainWindowHandle -ne [IntPtr]::Zero) { throw 'murmur mcp shows a window' }
        if ($replies[0].result.serverInfo.name -ne 'murmur') { throw 'murmur mcp did not answer initialize' }
        $tools = @($replies[1].result.tools.name)
        Write-Host "murmur mcp lists: $($tools -join ', ')"
        foreach ($tool in 'list_meetings', 'search', 'get_transcript', 'get_summary', 'recent_dictations') {
            if ($tools -notcontains $tool) { throw "murmur mcp lists no $tool tool" }
        }
        # The library the running app created answers, empty: no "open Murmur
        # once to finish updating", no missing database.
        if ($replies[2].result.isError -eq $true) { throw "list_meetings failed: $($replies[2].result.content[0].text)" }

        $server.StandardInput.Close()
        if (-not $server.WaitForExit(20000)) { throw 'murmur mcp did not exit when its stdin closed' }
        if ($server.ExitCode -ne 0) { throw "murmur mcp exited with code $($server.ExitCode)" }
    }
    finally {
        if (-not $server.HasExited) { $server.Kill() }
    }
}

# `murmur.exe --help` run with its output redirected, as a script or an
# assistant runs it: the console attach must leave those pipes alone. (The
# attach itself needs a real terminal and is not covered here.)
function Assert-CliOutput {
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $script:exe
    $info.Arguments = '--help'
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $cli = [System.Diagnostics.Process]::Start($info)
    $output = $cli.StandardOutput.ReadToEndAsync()
    if (-not $cli.WaitForExit(20000)) { $cli.Kill(); throw 'murmur --help did not exit within 20 s' }
    if ($cli.ExitCode -ne 0) { throw "murmur --help exited with code $($cli.ExitCode)" }
    $text = $output.Result
    if ($text -notmatch 'Murmur - Speech to Text' -or $text -notmatch '--transcribe-file') {
        throw "murmur --help printed to its pipe: $text"
    }
    Write-Host 'murmur --help printed to its redirected output'
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

    # An install that predates the move keeps its recordings in the Roaming
    # folder: the first launch moves them to Local app data.
    $legacyRecordings = Join-Path $roamingData 'recordings'
    New-Item -ItemType Directory -Force -Path $legacyRecordings | Out-Null
    [System.IO.File]::WriteAllBytes((Join-Path $legacyRecordings 'legacy-smoke.wav'), [byte[]](1, 2, 3, 4))

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
    if ($settings.settings.meetings_enabled -ne $true) { throw 'Meetings are not on in the settings of a new install' }
    $settings.settings.onboarding_completed = $true
    # The AI assistant server answers only when both switches are on.
    foreach ($switch in 'mcp_enabled', 'mcp_include_dictations') {
        $settings.settings | Add-Member -NotePropertyName $switch -NotePropertyValue $true -Force
    }
    $settings | ConvertTo-Json -Depth 32 | Set-Content -Encoding utf8NoBOM $settingsFile

    $murmur = Start-Murmur
    Save-Window $murmur 'settings-window.png'
    Assert-InsideWorkArea $murmur
    if (-not (Test-Path $meetingsDatabase)) { throw "Meetings are on but the app created no $meetingsDatabase" }
    # The heavy folders live in Local app data, which never roams; the small
    # files stay in Roaming.
    foreach ($heavy in 'recordings', 'meetings') {
        if (-not (Test-Path (Join-Path $localData $heavy))) { throw "No $heavy folder in $localData" }
        if (Test-Path (Join-Path $roamingData $heavy)) { throw "The $heavy folder is still in $roamingData" }
    }
    $movedRecording = Join-Path $localData 'recordings\legacy-smoke.wav'
    if (-not (Test-Path $movedRecording)) { throw 'The recording of the old install did not move to Local app data' }
    if ((Get-Item $movedRecording).Length -ne 4) { throw 'The moved recording changed' }
    if (-not (Test-Path $settingsFile)) { throw 'settings_store.json left the Roaming folder' }

    # A second instance forwards the toggle to the running one, then quits.
    # Without a microphone on the runner, the overlay reports it.
    Start-Process -FilePath $script:exe -ArgumentList '--toggle-transcription' -Wait
    Start-Sleep -Seconds 2
    Save-Desktop 'desktop-overlay.png'
    Start-Process -FilePath $script:exe -ArgumentList '--cancel' -Wait
    Start-Sleep -Seconds 3

    # The app is still running: the `mcp` path must not reach its single
    # instance plugin.
    Assert-McpServer
    Assert-CliOutput

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
