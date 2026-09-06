"""Export just the `tool` head, for the router to embed.

THE ROUTER DOES NOT NEED THE SLOTS. It needs to know whether a message is a
request one of these two tools answers, and which — three classes.

WHAT SHIPS IS 16384 BUCKETS, 48 KiB — `train_slots.py 16384 && export_gate.py`,
which reproduces `crates/tacet-tools/src/slot_gate.bin` byte for byte. This
paragraph used to describe 2048 buckets at 6 KiB "against 46 KiB for all six
heads", which was the FIRST version of the gate and is not what is in the tree:
that one scored 117 of 131 on the task cases and was rejected, because it was
trained without the other tools' work as negatives and called 38% of the other
suites' messages an extraction request. The shipped one scores 107 on the same
cases and does not. Both numbers are in `esp32/README.md` with the reason.

The file is `<u32 buckets><u32 classes>` then the int8 weights, row-major by
bucket. `slot_gate.rs` reads exactly that and nothing else.
"""
import struct, sys, numpy as np
from train_slots import HEADS

# THE BUCKET COUNT COMES FROM THE FILE, not from `train_slots.BUCKETS` — that
# constant is read out of sys.argv, which here is the OUTPUT PATH. Importing a
# module whose constants depend on the command line is a trap; this script only
# needs the header, so it reads it.
with open("slots.bin", "rb") as f:
    buckets, _ = struct.unpack("<II", f.read(8))
    W = {}
    for head in HEADS:
        nc, _ = struct.unpack("<II", f.read(8))
        W[head] = np.frombuffer(f.read(buckets * nc), dtype=np.int8).reshape(buckets, nc)

out = sys.argv[1] if len(sys.argv) > 1 else "../crates/tacet-tools/src/slot_gate.bin"
with open(out, "wb") as f:
    f.write(struct.pack("<II", buckets, len(HEADS["tool"])))
    f.write(W["tool"].tobytes())
print(f"{out}: {8 + buckets*len(HEADS['tool'])} bytes, {buckets} buckets, "
      f"classes {HEADS['tool']}")
