# echo-probe

Measures what macOS voice processing (`AVAudioInputNode.setVoiceProcessingEnabled`) does to the microphone of a meeting held on the Mac's speakers. Murmur does not ship it. Its numbers decide whether meetings record the microphone with voice processing on speakers.

The probe plays a voice through the default output from another process (`afplay`, standing in for the call app) and records the microphone three times:

1. Raw, with nothing else running.
2. With voice processing on, while a child process records the raw microphone at the same moment.
3. As 2, while you read a sentence aloud over the voice (double talk).

It reports:

- **Echo removed (ERLE)**: the raw microphone against the processed one, over the moments the other app's voice is heard. Gate: at least 15 dB.
- **Ducking**: how much quieter the other app's voice reaches the raw microphone while voice processing runs. Gate: at most 3 dB.
- **Double-talk recall**: the words of your sentence that the meetings helper hears in each recording, processed against raw. Gate: at least 95 %. Needs `--helper` and `--models-dir`.

## Run

The Mac plays sound for about two minutes. Use the built-in speakers at a normal volume, in a quiet room. macOS asks for microphone access for the terminal the first time.

```sh
swiftc -O scripts/echo-probe/main.swift -o /tmp/echo-probe
/tmp/echo-probe \
  --helper src-tauri/binaries/murmur-meetings-helper-aarch64-apple-darwin \
  --models-dir "$HOME/Library/Application Support/com.dailyxplorer.murmur/meetings/models"
```

The probe only reads the models. The recordings stay in a new `echo-probe-*` folder under the temporary folder, or under `--out`. Nothing is uploaded. The probe prints the output's transport and data source, never its name.
