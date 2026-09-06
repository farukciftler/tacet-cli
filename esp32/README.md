# A slot classifier that fits in an ESP32-S3

92 KiB of int8 beats a distilled 135M model at filling `search_filter`'s and
`message_intent`'s closed fields, on the same held-out cases, using 4,266
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

Scored on the **131 human-written cases** in `benchmarks/tasks/`. The generator's
templates are deliberately not the benchmark sentences, so this is generalisation
to another hand's phrasing, not memorisation — and **95 of those cases were
written after this model was trained**, without looking at what it gets wrong.

Two columns, because the training set changed and the difference is the
interesting part. The **1,894** column is the generator as it was when this page
was first written. The **2,306** column is the generator today: it also produces
the *other tools'* work as negatives — read a file, add two numbers, check a
repository — which is what `gen_slots.py` calls `other_examples`.

| head | 1,894 rows | 2,306 rows | size |
|---|---|---|---|
| `gate` | **124/131** | 111/131 | 8 KiB |
| `tool` | **119/131** | 100/131 | 12 KiB |
| `audience` | 123/131 | 123/131 | 20 KiB |
| `price` | 118/131 | 117/131 | 16 KiB |
| `when` | **123/131** | 120/131 | 16 KiB |
| `intent` | **116/131** | 109/131 | 20 KiB |

**The right-hand column is worse and it is the one that ships.** Both are
reproducible — `OTHER=0 python3 gen_slots.py 1200 train.jsonl` gives the left,
the default gives the right (measured 6 Sep 2026, M-series, both at 4096
buckets). What the negatives buy is not on this test set at all: without them the
`gate` head called **38% of the 709 messages** of the other suites — files,
arithmetic, git, memory — an extraction request, and wiring that into the router
cost fourteen top-three positions. This table is scored only on messages that
ARE extraction work, so it can only see the price and never the thing bought.
That is worth saying plainly rather than publishing the flattering column: a test
set that cannot see a defect will always prefer the model that has it.

For scale on the `tool` row: Tacet's own router reached these two tools on
**87 of the 105** cases that expect one, using a hand-written list of substring
triggers. It now reaches **102** — the difference is a 48 KiB head of this same
model, wired in as an additional signal and measured to cost the routing eval
nothing. See below.

Against the distilled 135M from [training/](../training/), on the same held-out
task cases it was scored on — **measured 5 Sep 2026 against the 1,894-row model
and not re-derived since**, so read it beside the left-hand column above:

| | SmolLM2-135M | this, 92 KiB |
|---|---|---|
| `search_filter` tool | 4/5 | **5/5** |
| `search_filter` slots | 1/5 | **15/15** |
| `message_intent` tool | 0/4 | **4/4** |
| `message_intent` intent | 0/4 | **3/4** |

Nine cases is a small denominator, and the comparison is only about the closed
fields — the 135M is doing a harder job, end to end, including the open-text
arguments this cannot touch.

`slots.c` counts its own operations: **4,266 per message** at a 48.9-byte mean
(1,098 hash, 3,168 accumulate). The ESP32-S3 figures below are that number
divided by documented characteristics of that part — **arithmetic, not
silicon**. No ESP32-S3 has run this. An **ESP8266 has**, and what it measured is
below the table.

| cycles/op | per message at 240 MHz | |
|---|---|---|
| 1.0 | 17.8 µs | optimistic: everything single-cycle |
| 2.5 | 44.4 µs | likely: int8 load and add, no SIMD |
| 5.0 | 88.9 µs | pessimistic: loop overhead and misses |

**One row of that guess has now been measured, on a different part.**
[device/](device/) runs the same two loops on a NodeMCU (ESP8266EX, Xtensa
LX106, 80 MHz, -O2) and compares all 23 accumulators against `slots.c` on every
message: **141 of 141 agree, 0 disagreements**, reproduced identically three
times. The cost there is **44.50 cycles per operation** — 188,576 cycles, 2,357
µs, 424 messages/s (measured 6 Sep 2026).

**That is nine times the pessimistic row, and it is not a refutation of it**,
because it is not the same regime: the table above assumes the 92 KiB sits in
internal SRAM, and an ESP8266 has 49 KiB of heap, so the weights live in flash
and every one of the ~3,300 weight reads per message pays `pgm_read_byte` and a
cache the buckets thrash. What the number prices is the claim at the top of this
page — *"at 92 KiB the weights are 18% of the internal SRAM, so the PSRAM
bandwidth wall never applies"*. That was the load-bearing assumption of the whole
argument and it had never been costed. On a part where the weights do not fit, the identical
code is an order of magnitude slower. The argument holds; it now has a
measurement under it rather than a claim.

**What is still not measured** is the half that would separate arithmetic from
memory. The op count does not move with the bucket width, so the same work run
against weights in DRAM isolates the memory term exactly — a 1,024-bucket build
(23 KiB) fits an ESP8266's heap and is compiled, but the board could not be
reflashed. Two causes, and only one yielded to software: esptool's reset drives
the auto-reset lines with two ioctls and passes through a state that boots the
sketch instead of the bootloader, which `device/enter_download.py` fixes; the
transfer then corrupts anyway, intermittently and in proportion to its length,
through the USB hub the board is reached by.
[device/README.md](device/README.md) says what that would take.

**These numbers replace 4,380 / 50 bytes / 18-46-91 µs, and the correction is the
kind this repository has a rule against needing.** Those were the numbers of the
**36**-message benchmark. `c14645e` grew it to 131 and re-derived the accuracy
tables, which still reproduce exactly — but never re-ran the op count, so a
measured figure kept a date and a decimal point while describing a set that no
longer existed. Worse, `budget.py` PRINTED "on the 36 benchmark messages" while
reading 131, so re-running the script to check this page gave a different answer
with nothing to explain it. It prints the count it actually read now.

Bucket count is the dial, and it costs accuracy as well as flash. Measured on the
shipping (2,306-row) training set, 6 Sep 2026 — the accuracy column is the `tool`
head, so the size beside it is that head's, and the last column is what all six
heads come to:

| buckets | `tool` head | `tool` correct | all six heads |
|---|---|---|---|
| 1024 | 3 KiB | — | 23 KiB |
| 2048 | 6 KiB | 88/131 | 46 KiB |
| 4096 | 12 KiB | 100/131 | **92 KiB** |
| 8192 | 24 KiB | 102/131 | 184 KiB |
| 16384 | **48 KiB** | 107/131 | 368 KiB |

The router does not embed all six heads: it takes the `tool` head alone at 16384
buckets, which is the **48 KiB** blob at `crates/tacet-tools/src/slot_gate.bin`.
`python3 train_slots.py 16384 && python3 export_gate.py` reproduces that file
byte for byte from this directory (verified 6 Sep 2026).

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

**And a third that this check did NOT catch, which is the more useful lesson.**
`\n`, `\r`, `\v` and `\f` were kept as characters on the C and Rust side where
Python's `str.split` collapses them — so every multi-line message hashed to
buckets the weights had never seen. It shipped, and it survived the check
because **no case in `benchmarks/tasks/` contains a newline**, while
`message_intent` exists to classify PASTED messages, where they are the rule. A
cross-check is only as good as the shapes it is given, so `check.py` now appends
messages carrying every shape the fold treats specially — newlines, CRLF, tabs
and vertical tabs, upper-case Turkish, an em dash, an emoji, a non-breaking
space, leading and trailing runs — and compares those too. Breaking the newline
case again turns it red.

Those messages travel to the C as one escaped line each and are keyed by INDEX
rather than by their own text: keying on the message meant a newline broke the
line-oriented output carrying it, so the harness failed where the fold was fine.

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

The board, if there is one — the same model, measured rather than divided:

```bash
cd device
python3 weights_header.py               # slots.bin -> slots_device/weights.h
pio run                                 # arm64 Linux container; see device/README.md
python3 device_check.py /dev/cu.usbserial-XXX
```

`slot_gate.rs` has tests that pin what the blob does on fixed strings. If they
fail after a regeneration the model changed; decide whether that was intended
rather than updating the expectation.

Needs numpy and a C compiler. Like [training/](../training/), it is outside the
Rust workspace on purpose: `cargo test` must never try to run any of it.
