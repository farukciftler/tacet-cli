"""Split every benchmark file into a TRAIN and a TEST half, by case.

WHY THIS EXISTS. The first distillation run generated its training set from
every benchmark file and then reported the student's "after" numbers on three
of those same files. 179 of the 1017 pairs came from the cases being scored.
The headline was measured on training data.

Splitting BY CASE rather than by file is the point. A by-file split (train on
everything except irrelevance.json) leaves the training set with zero examples
of correctly calling nothing and one slot-filling example — the two things the
first run was worst at — so the "generalisation test" would only be measuring
that the set had a hole in it. Counted: of 838 pairs outside those three files,
ABSTAIN 0, SLOT 1.

STRATIFIED PER FILE, not globally, because a global hash split gave one file 6
test cases and another 38. Each file contributes the same quarter of itself, so
every language and every tool group is represented on both sides.

The order is sha1 of the case name, so the assignment is stable across runs and
independent of file order and of Python's hash seed.
"""
import hashlib, json, os, sys, glob

SRC, OUT = sys.argv[1], sys.argv[2]
PCT = int(sys.argv[3]) if len(sys.argv) > 3 else 25

def rank(name):
    return hashlib.sha1(name.encode()).hexdigest()

n = {"train": 0, "test": 0}
merged = {"train": [], "test": []}
for f in sorted(glob.glob(os.path.join(SRC, "*", "*.json"))):
    rel = os.path.relpath(f, SRC)
    doc = json.load(open(f, encoding="utf-8"))
    if "cases" not in doc:
        continue
    order = sorted(doc["cases"], key=lambda c: rank(c["name"]))
    cut = round(len(order) * PCT / 100)
    halves = {"test": order[:cut], "train": order[cut:]}
    for half, cases in halves.items():
        if not cases:
            continue
        p = os.path.join(OUT, half, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        json.dump(dict(doc, cases=cases), open(p, "w", encoding="utf-8"),
                  ensure_ascii=False, indent=1)
        n[half] += len(cases)
        merged[half].append(doc.get("requires", []))
        for c in cases:
            merged[half].append(c)

# One merged file per half as well: 39 held-out cases spread over 21 files score
# nothing you can read, but the same cases as one composite do.
for half in ("train", "test"):
    reqs, cases = [], []
    for item in merged[half]:
        (reqs.extend(item) if isinstance(item, list) else cases.append(item))
    json.dump({"name": f"all-{half}", "language": "mixed",
               "requires": sorted(set(reqs)), "cases": cases},
              open(os.path.join(OUT, half, "ALL.json"), "w", encoding="utf-8"),
              ensure_ascii=False, indent=1)
print(f"train {n['train']} cases / test {n['test']} cases", flush=True)
