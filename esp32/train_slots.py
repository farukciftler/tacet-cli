"""A slot classifier small enough to live in an ESP32-S3's SRAM.

WHY A CLASSIFIER AND NOT A MODEL. `search_filter`'s `audience`, `price` and
`when` are `choice[...]` fields and `message_intent`'s `intent` is one of four.
A closed vocabulary is not a generation problem, it is an argmax over a handful
of classes — and the guarantee the grammar buys on a GPU ("a sixth value cannot
be emitted") is free here, because there is nothing to emit from.

FEATURES: hashed character n-grams, 3 to 5, over the lowercased message. Chosen
over words because the two languages that matter here agglutinate — "ücretsiz",
"ücretsizdir", "parasız" share no word but plenty of trigrams — and because a
hash table has a size you choose rather than a vocabulary that grows.

The model is one int8 weight per (bucket, class). Nothing is learned that cannot
be added with an integer accumulator.
"""
import json, os, sys, math, random, struct
import numpy as np

BUCKETS = int(sys.argv[1]) if len(sys.argv) > 1 else 4096
NGRAMS = (3, 4, 5)

HEADS = {
    "gate":     ["none", "tool"],
    "tool":     ["none", "search_filter", "message_intent"],
    "audience": ["none", "family", "kids", "adults", "seniors"],
    "price":    ["none", "free", "cheap", "premium"],
    "when":     ["none", "today", "tomorrow", "weekend"],
    "intent":   ["none", "promised_date", "dispute", "paid", "irrelevant"],
}

# THE UPPERCASE FORMS ARE MAPPED BEFORE `.lower()`, and that ordering is not
# cosmetic. Python lowercases `İ` to TWO codepoints — `i` followed by U+0307
# COMBINING DOT ABOVE — so a message containing it produced six more n-grams in
# the trainer than on the device, which maps the two UTF-8 bytes straight to
# `i`. The accumulator cross-check in check.py found it; comparing argmaxes
# never would have.
UPPER = {"İ":"i","I":"i","Ğ":"g","Ü":"u","Ş":"s","Ö":"o","Ç":"c","Â":"a","Î":"i"}
LOWER = {"ı":"i","ğ":"g","ü":"u","ş":"s","ö":"o","ç":"c","â":"a","î":"i"}

def fold(text):
    """Lowercase, and fold the Turkish letters the way the device does.

    ANYTHING STILL NON-ASCII AFTER THE MAPPING IS DROPPED, because that is what
    a device that only decodes the two-byte Turkish letters does with an em dash
    or an emoji. Keeping them here and dropping them there is a difference of a
    few n-grams, which is invisible in the accuracy and fatal to the guarantee
    that the two sides compute the same features.
    """
    t = "".join(UPPER.get(ch, ch) for ch in text).lower()
    for a, b in LOWER.items():
        t = t.replace(a, b)
    t = "".join(ch for ch in t if ch.isascii())
    return " " + " ".join(t.split()) + " "

def features(text):
    """Bucket indices for one message. FNV-1a, so the C side is four lines."""
    t = fold(text)
    idx = []
    b = t.encode("utf-8")
    for n in NGRAMS:
        for i in range(len(b) - n + 1):
            h = 2166136261
            for c in b[i:i+n]:
                h = ((h ^ c) * 16777619) & 0xFFFFFFFF
            idx.append(h % BUCKETS)
    return idx

def load(path):
    return [json.loads(l) for l in open(path, encoding="utf-8")]

def read_benchmark():
    """The TEST set: the human-written cases, labelled from their `evidence`."""
    rows = []
    repo = os.environ.get("TACET_REPO") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    for f, tool in ((f"{repo}/benchmarks/tasks/search_filter.json", "search_filter"),
                    (f"{repo}/benchmarks/tasks/message_intent.json", "message_intent")):
        d = json.load(open(f, encoding="utf-8"))
        for c in d["cases"]:
            for s in c["steps"]:
                ev = dict(e.split("=", 1) for e in s.get("evidence", []) if "=" in e)
                called = s.get("expect") is not None
                rows.append({
                    "text": s["message"],
                    "gate": "tool" if called else "none",
                    "tool": tool if called else "none",
                    "audience": ev.get("audience", "none"),
                    "price": ev.get("price", "none"),
                    "when": ev.get("when", "none"),
                    "intent": ev.get("intent", "none"),
                })
    return rows

def train_head(rows, head, epochs=25, lr=0.5):
    classes = HEADS[head]
    W = np.zeros((BUCKETS, len(classes)), dtype=np.float32)
    data = [(features(r["text"]), classes.index(r[head])) for r in rows]
    rng = random.Random(0)
    for ep in range(epochs):
        rng.shuffle(data)
        for idx, y in data:
            scores = W[idx].sum(axis=0)
            scores -= scores.max()
            p = np.exp(scores); p /= p.sum()
            p[y] -= 1.0                      # dL/dscore for softmax + CE
            np.add.at(W, idx, -lr * p / max(len(idx), 1))
    return W

def quantise(W):
    """int8 with one shared scale per head — the whole point is that the device
    does integer adds, and a per-head scale keeps the argmax identical."""
    scale = np.abs(W).max() / 127.0
    if scale == 0:
        scale = 1.0
    return np.clip(np.round(W / scale), -127, 127).astype(np.int8), scale

def predict(Wq, idx, classes):
    return classes[int(Wq[idx].sum(axis=0, dtype=np.int32).argmax())]

if __name__ == "__main__":
    train = load("train.jsonl")
    test = read_benchmark()
    print(f"train {len(train)} synthetic · test {len(test)} human-written\n")
    total_bytes = 0
    results = {}
    packed = {}
    for head in HEADS:
        W = train_head(train, head)
        Wq, scale = quantise(W)
        packed[head] = (Wq, scale)
        total_bytes += Wq.size
        ok = sum(predict(Wq, features(r["text"]), HEADS[head]) == r[head] for r in test)
        results[head] = (ok, len(test))
        print(f"  {head:9s} {ok:3d}/{len(test)}  {100*ok/len(test):5.1f}%   "
              f"{Wq.size/1024:6.1f} KiB")
    print(f"\n  total weights: {total_bytes/1024:.1f} KiB at {BUCKETS} buckets")
    with open("slots.bin", "wb") as f:
        f.write(struct.pack("<II", BUCKETS, len(HEADS)))
        for head in HEADS:
            Wq, scale = packed[head]
            f.write(struct.pack("<II", len(HEADS[head]), 0))
            f.write(Wq.tobytes())
    print(f"  slots.bin: {os.path.getsize('slots.bin')} bytes on flash")
