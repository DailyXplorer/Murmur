# Speaks a three-utterance dialogue with two of Windows' own voices, 16 kHz
# mono as the app records, into <Out>\u1.wav .. u3.wav, each with the words
# the analysis must find in <Out>\uN.txt. A voice of each gender when the
# runner has both; with one voice the folder holds a `one-voice` file and the
# tests do not check the speakers. French needs French voices: a Server image
# usually has none, and the script says so and writes nothing.
# ASCII text only: Windows PowerShell reads the script as ANSI.
param(
  [Parameter(Mandatory = $true)][ValidateSet("en", "fr")][string]$Language,
  [Parameter(Mandatory = $true)][string]$Out
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Speech

$culture = if ($Language -eq "fr") { "fr-FR" } else { "en-US" }
$voices = @((New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -eq $culture })
$voices | ForEach-Object { Write-Output "voice: $($_.VoiceInfo.Name) ($($_.VoiceInfo.Gender))" }
if ($voices.Count -eq 0) {
  if ($Language -eq "fr") {
    Write-Output "no fr-FR voice on this runner: the French dialogue is skipped"
    exit 0
  }
  throw "no $culture voice to record the dialogue with"
}
New-Item -ItemType Directory -Force $Out | Out-Null

$first = $voices[0]
$second = $voices | Where-Object { $_.VoiceInfo.Gender -ne $first.VoiceInfo.Gender } | Select-Object -First 1
if (-not $second) { $second = $voices | Where-Object { $_.VoiceInfo.Name -ne $first.VoiceInfo.Name } | Select-Object -First 1 }
if (-not $second) {
  $second = $first
  New-Item -ItemType File -Force (Join-Path $Out "one-voice") | Out-Null
}

if ($Language -eq "fr") {
  $utterances = @(
    @{ Voice = $first; Text = "Bonjour a tous, merci de participer a la reunion de planification aujourd'hui."; Words = "bonjour`nmerci`nparticiper`nreunion`nplanification`n" },
    @{ Voice = $second; Text = "Merci, j'ai relu le budget hier et les chiffres me semblent tout a fait raisonnables."; Words = "merci`nrelu`nbudget`nhier`nchiffres`nsemblent`nraisonnables`n" },
    @{ Voice = $first; Text = "Parfait, alors fixons la prochaine revue a jeudi apres-midi."; Words = "parfait`nfixons`nprochaine`nrevue`njeudi`n" }
  )
} else {
  $utterances = @(
    @{ Voice = $first; Text = "Good morning everyone, thank you for joining the planning meeting today."; Words = "morning`neveryone`nthank`njoining`nplanning`nmeeting`ntoday`n" },
    @{ Voice = $second; Text = "Thanks, I reviewed the budget yesterday and the numbers look quite reasonable."; Words = "thanks`nreviewed`nbudget`nyesterday`nnumbers`nlook`nquite`nreasonable`n" },
    @{ Voice = $first; Text = "Great, then let us schedule the next review for Thursday afternoon."; Words = "great`nschedule`nnext`nreview`nthursday`nafternoon`n" }
  )
}

$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
$number = 0
foreach ($utterance in $utterances) {
  $number++
  $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
  $synth.SelectVoice($utterance.Voice.VoiceInfo.Name)
  $synth.SetOutputToWaveFile((Join-Path $Out "u$number.wav"), $format)
  $synth.Speak($utterance.Text)
  $synth.Dispose()
  [System.IO.File]::WriteAllText((Join-Path $Out "u$number.txt"), $utterance.Words)
}
