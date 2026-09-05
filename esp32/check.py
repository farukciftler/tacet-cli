"""The device and the trainer must compute the same thing.

A hashed-feature model has no way of complaining when the two sides disagree:
the C fold drops a letter the Python fold keeps, the n-grams differ, the buckets
differ, and the only symptom is an accuracy that is merely worse. So the two
implementations are run over every benchmark message and their argmaxes are
compared class by class. If this file passes, the numbers measured in Python are
the numbers the device would produce.
"""
import json, os, subprocess, sys
import numpy as np
from train_slots import HEADS, BUCKETS, features, read_benchmark
import struct

def load_bin(path="slots.bin"):
    with open(path, "rb") as f:
        buckets, nheads = struct.unpack("<II", f.read(8))
        W = {}
        for head in HEADS:
            nc, _ = struct.unpack("<II", f.read(8))
            W[head] = np.frombuffer(f.read(buckets * nc), dtype=np.int8).reshape(buckets, nc)
    return buckets, W

buckets, W = load_bin()
# THE TRAINER'S BUCKET COUNT AND THE FILE'S MUST AGREE. `features()` hashes
# modulo the module-level BUCKETS, so checking a slots.bin trained at a different
# width would compare two models and blame the C.
assert buckets == BUCKETS, (
    f"slots.bin has {buckets} buckets, train_slots.py is set to {BUCKETS}; "
    f"re-run `python3 train_slots.py {buckets}` or check with that width")
rows = read_benchmark()
msgs = [r["text"] for r in rows]

out = subprocess.run(["./slots", "1"], input="\n".join(msgs) + "\n",
                     capture_output=True, text=True,
                     env=dict(os.environ, DUMP="1")).stdout
NCLASS = sum(len(v) for v in HEADS.values())
c_raw = {}
for line in out.splitlines():
    parts = line.split("\t")
    if len(parts) != 2 + NCLASS:
        continue
    c_raw[parts[0]] = (int(parts[1]), [int(x) for x in parts[2:]])

mismatch = 0
for r in rows:
    idx = features(r["text"])
    py_acc = []
    for h in HEADS:
        py_acc.extend(int(v) for v in W[h][idx].sum(axis=0, dtype=np.int32))
    got = c_raw.get(r["text"])
    if got is None:
        print("C produced no row for:", r["text"][:60]); mismatch += 1; continue
    c_ngrams, c_acc = got
    if c_ngrams != len(idx):
        mismatch += 1
        print(f"NGRAM COUNT: {r['text'][:44]!r} python {len(idx)} vs c {c_ngrams}")
        continue
    if c_acc != py_acc:
        mismatch += 1
        d = [i for i, (a, b) in enumerate(zip(py_acc, c_acc)) if a != b]
        print(f"ACCUMULATORS: {r['text'][:44]!r} differ in {len(d)} of {NCLASS}")
print(f"\n{len(rows)} messages, {mismatch} disagreements between the C and the trainer")

# And the accuracy, from the C side's own accumulators.
score = {h: 0 for h in HEADS}
for r in rows:
    if r["text"] not in c_raw:
        continue
    acc = c_raw[r["text"]][1]
    off = 0
    for h in HEADS:
        n = len(HEADS[h])
        best = max(range(n), key=lambda c: acc[off + c])
        if HEADS[h][best] == r[h]:
            score[h] += 1
        off += n
print()
for h in HEADS:
    print(f"  {h:9s} {score[h]:3d}/{len(rows)}  {100*score[h]/len(rows):5.1f}%")
sys.exit(1 if mismatch else 0)
