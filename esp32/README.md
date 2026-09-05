# A slot classifier that fits in an ESP32-S3

92 KiB of int8 beats a distilled 135M model at filling `search_filter`'s and
`message_intent`'s closed fields, on the same held-out cases, using 4,380
integer operations per message.

## Why a classifier and not a model

`search_filter`'s `audience`, `price` and `when` are `choice[...]` fields;
`message_intent`'s `intent` is one of four. **A closed vocabulary is not a
generation problem.** It is an argmax over a handful of classes — and the
guarantee the grammar buys on a GPU, that a sixth value cannot be emitted, is
free here, because there is nothing to emit from.

That matters on this hardware because a decode step reads every weight once, so
`tokens/s <= bandwidth / model size`, and an ESP32-S3's external PSRAM sustains
roughly 40 MB/s:

| model | Q4 weights | fits? | ceiling |
|---|---|---|---|
| TinyStories-15M | 8 MB | flash | 5.33 tok/s |
| SmolLM2-135M | 68 MB | **no** | 0.59 tok/s |
| FunctionGemma-270M | 135 MB | **no** | 0.30 tok/s |
| Qwen3-0.6B | 300 MB | **no** | 0.13 tok/s |
| **this** | **92 KiB** | **internal SRAM** | ~22,000 messages/s |

At 92 KiB the weights are 18% of the internal SRAM, so the PSRAM bandwidth wall
never applies — which is the whole reason the last row is four orders of
magnitude from the others rather than merely smaller.

## What it does and does not do

Six heads: `gate` (does this need a tool at all), `tool`, and the four closed
slots. It does **not** extract `city`, `promised_date` or `amount` — those are
open text, they are span copying rather than classification, and they stay with
whatever host the device talks to.

Features are hashed character n-grams, 3 to 5, over the folded message. Chosen
over words because the two languages here agglutinate — *ücretsiz*, *ücretsizdir*
and *parasız* share no word but plenty of trigrams — and because a hash table has
a size you choose rather than a vocabulary that grows. The model is one int8
weight per (bucket, class); nothing is learned that an integer accumulator
cannot add.

## Measured

Trained on 1,894 generated examples, scored on the **131 human-written cases** in
`benchmarks/tasks/`. The generator's templates are deliberately not the benchmark
sentences, so this is generalisation to another hand's phrasing, not
memorisation — and **95 of those cases were written after this model was
trained**, without looking at what it gets wrong.

| head | correct | size |
|---|---|---|
| `gate` | 124/131 | 8 KiB |
| `tool` | 119/131 | 12 KiB |
| `audience` | 123/131 | 20 KiB |
| `price` | 118/131 | 16 KiB |
| `when` | 123/131 | 16 KiB |
| `intent` | 116/131 | 20 KiB |

For scale on the `tool` row: Tacet's own router reached these two tools on
**87 of the 105** cases that expect one, using a hand-written list of substring
triggers. It now reaches **102** — the difference is a 48 KiB head of this same
model, wired in as an additional signal and measured to cost the routing eval
nothing. See below.

Against the distilled 135M from [training/](../training/), on the same held-out
task cases it was scored on:

| | SmolLM2-135M | this, 92 KiB |
|---|---|---|
| `search_filter` tool | 4/5 | **5/5** |
| `search_filter` slots | 1/5 | **15/15** |
| `message_intent` tool | 0/4 | **4/4** |
| `message_intent` intent | 0/4 | **3/4** |

Nine cases is a small denominator, and the comparison is only about the closed
fields — the 135M is doing a harder job, end to end, including the open-text
arguments this cannot touch.

`slots.c` counts its own operations: **4,380 per message** at a 50-byte mean
(1,127 hash, 3,253 accumulate). The device figures below are that number divided
by documented ESP32-S3 characteristics — **arithmetic, not silicon**. Nothing
here has been run on a board.

| cycles/op | per message at 240 MHz | |
|---|---|---|
| 1.0 | 18 µs | optimistic: everything single-cycle |
| 2.5 | 46 µs | likely: int8 load and add, no SIMD |
| 5.0 | 91 µs | pessimistic: loop overhead and misses |

Bucket count is the dial: 1024 buckets is 23 KiB, 4096 is 92 KiB, 8192 is
184 KiB.

## The trainer and the device must compute the same thing

`check.py` runs both implementations over every benchmark message and compares
them. This is the file that matters, because a hashed-feature model has no way
of complaining when the two sides disagree: a fold that drops a letter on one
side produces different n-grams, different buckets, and the only symptom is an
accuracy that is merely worse.

**Comparing the argmax was not enough.** Breaking the fold so `ı` stops folding
to `i` changes the features of every Turkish message in the set and flipped
*not a single* prediction — a guard that survives its own defect. Comparing every
accumulator instead found two real bugs immediately:

* Python lowercases `İ` to **two** codepoints, `i` plus U+0307 COMBINING DOT
  ABOVE, where the device maps two UTF-8 bytes straight to `i`. Six n-grams of
  difference on any message containing it.
* Reading every non-ASCII character as two bytes takes an em dash for a Turkish
  letter and then resumes mid-sequence. The lead byte gives the length; only the
  two-byte Turkish letters are mapped and every other sequence is skipped whole,
  on both sides.

## Two models, because the negatives depend on where it runs

The same generator and trainer produce a second, differently-shaped model that
ships inside Tacet's router as `crates/tacet-tools/src/slot_gate.bin`. The
difference is one switch, and it is worth understanding before regenerating
either.

**A device whose only job is these two requests never sees "open report.docx".**
Training on such negatives costs capacity and buys nothing: the `tool` head goes
from 119/131 to 100/131 at the same 4096 buckets. So the device model is
`OTHER=0`, and that is what every table above is measured on.

**The router sits among forty-seven tools and does see them.** Without those
negatives it calls **38% of the other suites' 709 messages** an extraction
request, and wiring it in cost the routing eval fourteen of its top-three
positions. With them the false-positive rate is 7.6%, the eval is untouched at
166/166, and the gate catches **15 of the 18** requests the router's hand-written
triggers cannot reach at all. It needs 16384 buckets to hold both jobs, but only
one head ships — 48 KiB.

## Running it

```bash
cd esp32
python3 extract_msgs.py > msgs.txt      # the benchmark messages

# the device model — every table above
OTHER=0 python3 gen_slots.py 1200 train.jsonl
python3 train_slots.py 4096             # trains, scores, writes slots.bin
cc -O2 -o slots slots.c
python3 check.py                        # device vs trainer — must print 0 disagreements
python3 budget.py                       # the tables above

# the router gate — one head, with the other tools' work as negatives
OTHER=1 python3 gen_slots.py 1200 train.jsonl
python3 train_slots.py 16384
python3 export_gate.py                  # -> crates/tacet-tools/src/slot_gate.bin
```

`slot_gate.rs` has tests that pin what the blob does on fixed strings. If they
fail after a regeneration the model changed; decide whether that was intended
rather than updating the expectation.

Needs numpy and a C compiler. Like [training/](../training/), it is outside the
Rust workspace on purpose: `cargo test` must never try to run any of it.
