"""Checks what `murmur-meetings-helper analyze` printed for the smoke
dialogue: words in time order, at least two speakers, and `done`.

usage: check_analysis.py <helper output>
"""
import json
import sys


def main(path):
    messages = [json.loads(line) for line in open(path) if line.strip()]
    kinds = [message["type"] for message in messages]
    errors = [message for message in messages if message["type"] == "error"]
    assert not errors, f"helper reported errors: {errors}"
    assert kinds[-1] == "done", "the helper did not finish"

    words = next(m for m in messages if m["type"] == "words")["words"]
    assert len(words) >= 40, f"only {len(words)} words heard"
    starts = [word["s"] for word in words]
    assert starts == sorted(starts), "word times go backwards"

    turns = next(m for m in messages if m["type"] == "diarization")["segments"]
    speakers = {turn["spk"] for turn in turns}
    assert len(speakers) >= 2, f"speakers found: {speakers}"
    print(f"ok: {len(words)} words, {len(speakers)} speakers, {len(turns)} turns")


if __name__ == "__main__":
    main(sys.argv[1])
