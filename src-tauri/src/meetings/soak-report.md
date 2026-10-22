# Meetings soak report

Written by `meetings_soak` (`src-tauri/src/meetings/soak.rs`). Run it on a Mac with the models downloaded:

```sh
cd src-tauri && cargo test --locked meetings_soak -- --ignored --nocapture
```

- Machine: Apple M4 Pro, 25 GB RAM, macOS 27.0.1
- Commit: 1edf29e6
- Build: debug test binary; the meetings helper is the release build `build.rs` makes
- Audio: 0:09:12 of French dialogue said by `say` (Thomas on the microphone, Amélie on system audio and at -20 dB, 40 ms late, on the microphone), looped to 3 hours with 200 to 500 ms more silence each time round
- Codex: a local fake that answers each piece after 2 s with about as many words as it holds, so alignment and echo removal work on text that does not match the audio; the summary is canned

## Runs

| | A: 3 h, no stop | C: B's meeting after both kills |
|---|---|---|
| Audio processed | 3:00:00 | 1:30:00 |
| Peak footprint of processing, as Activity Monitor shows it | 1.56 GiB | 0.64 GiB |
| Peak footprint of processing, freed memory handed back at once | 0.29 GiB | not run |
| Peak resident memory, meetings helper | 1.43 GiB | 0.84 GiB |
| Analysis (helper) | 89.1 s | 37.0 s |
| Transcription | 55.5 s | 15.1 s |
| Aligning | 42.4 s | 20.8 s |
| Compression | 12.5 s | 6.4 s |
| Whole relaunch | 171.5 s | 79.3 s |
| Speakers | Some(Separated) | Some(Separated) |
| Bubbles per speaker | {"me": 761, "s1": 749} | {"me": 431, "s1": 427} |
| Echoes removed | 446 | 265 |

Run A recorded 3 hours in 110 s and dropped 0 ms of audio.

Run B was killed (SIGKILL) at 1:30:00 of recording, after 5400100 ms had been pushed to each track. At the relaunch the microphone track held 5400048 ms and system audio 5399824 ms.

Run C: the first relaunch was killed once 10 pieces were transcribed. The next one sent 10 pieces, 0 of them already transcribed.

Memory: each piece sent to Codex takes short-lived buffers (its samples and copies of them, the WAV and the upload body: over 100 MB for 10 minutes), freed once the piece is done. macOS's allocator keeps freed blocks that large in the footprint (its large cache and deferred reclaim) until the kernel takes them back, so the first row grows with the number of pieces and falls again after transcription. The second row is the same processing of run A run with `MallocLargeCache=0 MallocDeferredReclaim=0`, which hands freed memory back at once: what processing holds, and what the 1 GiB threshold checks.

Timings as logged:

- A: `INFO Meeting 1 timings: audio 180.0 min, reading 27.1 s, transcription 55.5 s, analysis 89.1 s, aligning 42.4 s, summary 0.0 s, compression 12.5 s, total 171.5 s (63x real time)`
- A, freed memory handed back: `INFO Meeting 1 timings: audio 180.0 min, reading 29.3 s, transcription 56.0 s, analysis 76.0 s, aligning 42.9 s, summary 0.0 s, compression 12.7 s, total 161.2 s (67x real time)`
- C: `INFO Meeting 1 timings: audio 90.0 min, reading 14.9 s, transcription 15.1 s, analysis 37.0 s, aligning 20.8 s, summary 0.0 s, compression 6.4 s, total 79.3 s (68x real time)`

## Thresholds

- [x] A is ready
- [x] A: processing holds under 1 GiB
- [x] A: helper under 3 GiB
- [x] A: analysis under 10 min
- [x] A: both speakers present
- [x] A: no audio dropped
- [x] B: each track kept all but 1 s of what was pushed
- [x] C: the interrupted meeting is processed at relaunch
- [x] C: no transcribed piece sent again
- [x] C is ready
- [x] C: helper under 3 GiB
- [x] C: both speakers present

Every threshold passes: the helper keeps reading each track whole, with no windows.
