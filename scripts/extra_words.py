#!/usr/bin/env python3
"""Builds data/<lang>_extra.txt: rarer word forms that only mark a word as known.

Source: full FrequencyWords lists (OpenSubtitles 2018). Keeps words of at least
MIN_LEN letters seen at least MIN_COUNT times that the 50k lists lack.
The Ukrainian corpus is ~10x smaller and mixed with Russian, so it gets a lower
threshold and drops words found in the Russian 50k list.
Short words are left out on purpose: the full lists are full of short junk
("dfc", "ldf") that would block real layout fixes.

Usage: scripts/extra_words.py [DIR_WITH_<lang>_full.txt]   (downloads them if omitted)
"""
import os
import sys
import tempfile
import urllib.request

MIN_LEN = 5
MIN_COUNT = {"en": 15, "ru": 15, "uk": 5}
URL = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/{0}/{0}_full.txt"
ALPHABETS = {
    "en": set("abcdefghijklmnopqrstuvwxyz"),
    "ru": set("абвгдеёжзийклмнопрстуфхцчшщъыьэюя"),
    "uk": set("абвгґдеєжзиіїйклмнопрстуфхцчшщьюя"),
}

root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "data")
src = sys.argv[1] if len(sys.argv) > 1 else tempfile.mkdtemp()


def top_words(lang):
    with open(os.path.join(root, f"{lang}_50k.txt"), encoding="utf-8") as f:
        return {line.split()[0].lower() for line in f if line.strip()}


for lang, alphabet in ALPHABETS.items():
    full = os.path.join(src, f"{lang}_full.txt")
    if not os.path.exists(full):
        urllib.request.urlretrieve(URL.format(lang), full)
    known = top_words(lang) | (top_words("ru") if lang == "uk" else set())
    out, seen = [], set()
    with open(full, encoding="utf-8") as f:
        for line in f:
            parts = line.split()
            if len(parts) != 2:
                continue
            word, count = parts[0].lower(), int(parts[1])
            if count < MIN_COUNT[lang]:
                break
            if len(word) >= MIN_LEN and set(word) <= alphabet and word not in known and word not in seen:
                seen.add(word)
                out.append(word)
    with open(os.path.join(root, f"{lang}_extra.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print(f"{lang}: {len(out)} words")
