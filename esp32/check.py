"""The trainer, the device and the SHIPPED RUST must compute the same thing.

A hashed-feature model has no way of complaining when the two sides disagree:
the C fold drops a letter the Python fold keeps, the n-grams differ, the buckets
differ, and the only symptom is an accuracy that is merely worse. So the two
implementations are run over every benchmark message and their argmaxes are
compared class by class. If this file passes, the numbers measured in Python are
the numbers the device would produce.
"""
import json, os, subprocess, sys
import numpy as np
from train_slots import HEADS, BUCKETS, features, fold, read_benchmark
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

# MESSAGES THE BENCHMARK DOES NOT CONTAIN, because the fold bug that shipped
# survived this very check: no case in `benchmarks/tasks/` has a newline in it,
# so `\n` being kept as a character on one side and collapsed on the other was
# invisible. `message_intent` classifies PASTED messages; multi-line input is
# the rule there. Every shape the fold treats specially gets one row here.
from shapes import SHAPES   # the list lives there so device_check.py shares it
# An EMPTY message is tested in slot_gate.rs instead: it cannot survive a
# line-oriented transport, and inventing a sentinel for it would test the
# harness rather than the fold.
rows = rows + [{"text": t, "gate": "none", "tool": "none", "audience": "none",
                "price": "none", "when": "none", "intent": "none"} for t in SHAPES]
msgs = [r["text"] for r in rows]

def escape(t):
    """One line per message; slots.c reverses this."""
    return (t.replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r")
             .replace("\t", "\\t").replace("\v", "\\v").replace("\f", "\\f"))

out = subprocess.run(["./slots", "1"], input="\n".join(escape(m) for m in msgs) + "\n",
                     capture_output=True, text=True,
                     env=dict(os.environ, DUMP="1")).stdout
NCLASS = sum(len(v) for v in HEADS.values())
c_raw = {}
for line in out.splitlines():
    parts = line.split("\t")
    if len(parts) != 2 + NCLASS:
        continue
    c_raw[int(parts[0])] = (int(parts[1]), [int(x) for x in parts[2:]])

mismatch = 0
for row_i, r in enumerate(rows):
    idx = features(r["text"])
    py_acc = []
    for h in HEADS:
        py_acc.extend(int(v) for v in W[h][idx].sum(axis=0, dtype=np.int32))
    got = c_raw.get(row_i)
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

# ---------------------------------------------------------------- the Rust
# THE THIRD IMPLEMENTATION, and the one that ships. Until this block existed the
# cross-check compared the trainer against the microcontroller and left the fold
# every Tacet user runs to seven fixed strings — one of which asserted a value
# the trainer does not produce, under a test named `the_fold_matches_the_trainer`.
def rust_folds(messages):
    out = subprocess.run(
        ["cargo", "run", "-q", "-p", "tacet-tools", "--example", "fold_dump"],
        cwd=os.path.dirname(os.path.abspath(__file__)) + "/..",
        input="\n".join(escape(m) for m in messages) + "\n",
        capture_output=True, text=True,
    )
    if out.returncode != 0:
        print("could not run the Rust fold:\n" + out.stderr[-800:])
        return None
    return [unescape(l) for l in out.stdout.splitlines()]


def unescape(s):
    r = []
    i = 0
    while i < len(s):
        if s[i] != "\\":
            r.append(s[i]); i += 1; continue
        n = s[i + 1] if i + 1 < len(s) else "\\"
        if n == "x":
            r.append(chr(int(s[i + 2:i + 4], 16))); i += 4
        else:
            r.append({"n": "\n", "r": "\r", "t": "\t", "v": "\v", "f": "\f",
                      "\\": "\\"}.get(n, n)); i += 2
    return "".join(r)


rust = rust_folds(msgs)
if rust is None:
    mismatch += 1
elif len(rust) != len(msgs):
    print(f"the Rust fold returned {len(rust)} lines for {len(msgs)} messages")
    mismatch += 1
else:
    rust_bad = 0
    for m, got in zip(msgs, rust):
        want = fold(m)
        if got != want:
            rust_bad += 1
            print(f"RUST FOLD: {m[:44]!r}\n    trainer {want!r}\n    rust    {got!r}")
    print(f"{len(msgs)} messages, {rust_bad} disagreements between the Rust and the trainer")
    mismatch += rust_bad

# And the accuracy, from the C side's own accumulators — over the BENCHMARK
# cases only. The shapes above exercise the fold, they are not labelled data.
score = {h: 0 for h in HEADS}
for row_i, r in enumerate(rows[: len(rows) - len(SHAPES)]):
    if row_i not in c_raw:
        continue
    acc = c_raw[row_i][1]
    off = 0
    for h in HEADS:
        n = len(HEADS[h])
        best = max(range(n), key=lambda c: acc[off + c])
        if HEADS[h][best] == r[h]:
            score[h] += 1
        off += n
print()
for h in HEADS:
    n_bench = len(rows) - len(SHAPES)
    print(f"  {h:9s} {score[h]:3d}/{n_bench}  {100*score[h]/n_bench:5.1f}%")
sys.exit(1 if mismatch else 0)
