# Fixtures of the "Import audio" decoder tests

A 440 Hz sine, 0.4 s long, each in another container, codec and rate, so a
test checks that the decoder reads it, finds its length, and brings it to
16 kHz mono with the tone intact. Nothing here is a recording of anyone.

| File                            | Made with                                                                                                   |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `tone-44k-stereo.mp3`           | `ffmpeg -f lavfi -i sine=frequency=440:duration=0.4:sample_rate=44100 -ac 2 -c:a libmp3lame -b:a 32k`       |
| `tone-24k-mono.flac`            | `ffmpeg -f lavfi -i sine=frequency=440:duration=0.4:sample_rate=24000 -ac 1 -c:a flac -compression_level 8` |
| `tone-44k-stereo-aac.m4a`       | `ffmpeg … sample_rate=44100 -ac 2 -c:a aac -b:a 32k`                                                        |
| `tone-22k-mono-alac.m4a`        | `ffmpeg … sample_rate=22050 -ac 1 -c:a alac`                                                                |
| `tone-8k-mono-alac.caf`         | `afconvert -f caff -d alac` on an 8 kHz mono WAV                                                            |
| `tone-24k-mono-aac.caf`         | `afconvert -f caff -d aac -b 24000` on a 44.1 kHz mono WAV (it picks 24 kHz)                                |
| `tone-8k-mono-pcm.caf`, `.aiff` | `afconvert -f caff -d LEI16` / `-f AIFF -d BEI16` on an 8 kHz mono WAV                                      |

WAV needs no file: the tests write several variants (8, 16, 24 and 32-bit
float; mono and stereo; 8 to 48 kHz) with `hound`.
