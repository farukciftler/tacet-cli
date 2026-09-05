"""Export just the `tool` head, for the router to embed.

THE ROUTER DOES NOT NEED THE SLOTS. It needs to know whether a message is a
request one of these two tools answers, and which — three classes. At 2048
buckets that is 6 KiB, against 46 KiB for all six heads, and the measured cost
is two cases of 131 against the 4096-bucket model.

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
