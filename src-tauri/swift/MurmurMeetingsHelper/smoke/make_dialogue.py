"""Writes a fictional two-voice dialogue as a 16 kHz mono WAV with macOS
`say`, for the meetings helper smoke test.

usage: make_dialogue.py <out.wav> <voice A> <voice B>
"""
import os
import subprocess
import sys
import tempfile
import wave

LINES = [
    "Good morning everyone, let's start the weekly project review.",
    "Morning. I prepared the update on the budget.",
    "Great. Where are we with the billing platform?",
    "The migration is done, only the load tests are left.",
    "Fine. Can we ship next Thursday?",
    "Yes, if the client team signs off on the scenarios before Wednesday.",
    "Perfect, I will follow up with them today.",
    "Thanks. Last point, the offsite moves to March.",
]
RATE = 16000
GAP_SECONDS = 0.6


def main(out_path, voices):
    frames = b""
    with tempfile.TemporaryDirectory() as tmp:
        for index, text in enumerate(LINES):
            aiff = os.path.join(tmp, f"{index}.aiff")
            wav = os.path.join(tmp, f"{index}.wav")
            subprocess.run(["say", "-v", voices[index % 2], "-o", aiff, text], check=True)
            subprocess.run(
                ["afconvert", "-f", "WAVE", "-d", f"LEI16@{RATE}", "-c", "1", aiff, wav],
                check=True,
            )
            with wave.open(wav) as reader:
                frames += reader.readframes(reader.getnframes())
            frames += b"\x00\x00" * int(GAP_SECONDS * RATE)
    with wave.open(out_path, "wb") as writer:
        writer.setnchannels(1)
        writer.setsampwidth(2)
        writer.setframerate(RATE)
        writer.writeframes(frames)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2:4])
