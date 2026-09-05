# Distilling a tool-caller

A 135M model cannot call tools in Tacet's format. After three minutes of
training on a set Tacet generated from its own benchmarks, it can — and the
number that says so is measured on cases the student never saw.

This directory is the recipe. It is deliberately small and deliberately outside
the Rust workspace: training needs PyTorch and a GPU, which this project does not
take a dependency on, and `cargo test` must never try to run any of it.

## Split first. This is not optional.

```bash
python3 training/split.py benchmarks /somewhere/split 25
# train 569 cases / test 189 cases
```

**THE FIRST RUN OF THIS RECIPE DID NOT DO THIS, AND ITS NUMBERS WERE WRONG.**
The set was generated from every benchmark file, and the student was then scored
on three of those same files. 179 of the 1017 pairs came from the cases being
scored. The headline — 0% to 61.5% correct calls — was measured partly on the
student's own training data, and it sat on the front page for a day.

`split.py` divides each file's cases into a train and a test half, **stratified
per file** and keyed on a hash of the case name so the assignment is stable
across runs. Per file rather than globally, because a single global hash gave one
file 6 test cases and another 38. It also writes an `ALL.json` per half: 165
held-out cases spread over 21 files score nothing you can read, but the same
cases as one composite do.

Splitting **by case** rather than by file matters more than it looks. Holding out
whole files (train on everything except `irrelevance.json`) leaves the training
set with no examples of the behaviour being tested, and the "generalisation test"
then only measures that the set had a hole in it.

## Where the data comes from

Not from a hand-written dataset, and not from a bigger model's raw output — most
of which is wrong. It comes from **the subset of a teacher's output that a
benchmark scored as correct**, over the TRAIN half only:

```bash
for f in /somewhere/split/train/*/*.json; do
  TACET_DISTIL_DIR=/somewhere/distil \
  TACET_WEB_CASSETTE=$PWD/benchmarks/cassettes \
    tacet bench run "$f" --model qwen3-4b --json --skip-missing
done
```

Every step that PASSES writes `{"case", "prompt", "completion"}` as JSONL. A step
that called the wrong tool, or called nothing when it should have called
something, contributes nothing — its prompt is exactly the input where the
student must *not* copy the teacher. "Correct" is the benchmark's own pass/fail,
never a judge model. Measured: Qwen3-4B over the 569 train cases gave **865
pairs**, 851 of them usable.

## What is actually in the set, and how to count it wrong

The set is four different behaviours in one pile, and they are not the same
lesson:

| | | rows |
|---|---|---|
| `TOOL` | a call to an ordinary tool | 261 |
| `ANSWER` | the last step of a turn: a tool result is already in the prompt, and the right answer is prose | 346 |
| `ABSTAIN` | tools were offered and calling none was correct | 186 |
| `SLOT` | a call to `search_filter` or `message_intent`, where the arguments are the whole task | 58 |

**COUNTING `ABSTAIN` IS WHERE THIS GOES WRONG.** The obvious test — does the
prompt contain `<tool_response>` — matches **every row**, because the system
prompt names the tag while explaining it (*"If a `<tool_response>` block answers
what was asked…"*). It reports 0 abstentions where there are 186, and a balancing
pass built on that number oversamples answer-composition turns 4x while believing
it is teaching restraint, which is the exact opposite of the intended repair. A
tool result is only real when a **user turn opens with the tag**, so the marker
has to be the whole `<|im_start|>user\n<tool_response>`, which the system prompt
cannot contain.

| marker | ANSWER | ABSTAIN | TOOL | SLOT |
|---|---|---|---|---|
| `<tool_response>` | 532 | **0** | 261 | 58 |
| user turn + the tag | 346 | **186** | 261 | 58 |

**`SLOT` WAS 13 ROWS AND NO WEIGHTING FIXED IT.** That number is why
`benchmarks/tasks/` went from 36 cases to 131 and why the router got a learned
gate: the teacher can only contribute a row for a case it passes, and it can only
pass a case whose tool the router actually shows it. Widening the suite and
carrying the teacher to those tools on 102 of 105 requests instead of 47 took
`SLOT` from **13 to 58**. Neither change alone would have done it, and no amount
of oversampling thirteen rows would have done it at all.

## The one constraint that is easy to miss

**The prompt is stored RENDERED, so teacher and student must share a chat
template.** That is what makes the set usable as-is, and also its limit: a Gemma
student trained on ChatML prompts would be trained on a format Tacet never shows
it. Qwen3 and SmolLM2 are both ChatML, which is why the run below pairs them.

SmolLM2 converts to a GGUF whose architecture is `llama`, which Tacet loads —
and refuses if the tokenizer carries no `<|im_start|>`, because `llama` names a
family that does not agree on a template.

## What it bought, on cases the student never saw

189 held-out cases, RTX 3090, 5 Sep 2026. Training is 292 s unweighted and 362 s
composed.

| SmolLM2-135M | base | defaults | `W_ABSTAIN=2 W_SLOT=3 CAP_ANSWER=1` |
|---|---|---|---|
| composite | 46.4 | **59.9** | 60.0 |
| irrelevance | **41/44** | 36/44 | 36/44 |
| tool selection | 6/119 | 51/119 | **52/119** |
| correct call | 0.0% | **36.4%** | 27.3% |
| `search_filter` tool / answer | 0/17 · 0/17 | 15/17 · 1/17 | **16/17** · **3/17** |
| `message_intent` tool | 0/10 | 1/10 | 2/10 |
| decode | 130 tok/s | 120–127 | 123–126 |
| peak resident | 529 MiB | 528 MiB | 529 MiB |
| training | — | 292 s | 362 s |

**COMPOSING THE SET HAS STOPPED BEING WORTH MUCH, AND THAT IS THE RESULT.** On
the older set — 36 task cases, 13 slot rows, 750 pairs — the same weighting was
worth 7.5 points of composite (54.9 against 62.4). Here it is worth 0.1. It was
compensating for a set with a hole in it; fill the hole and the compensation is
noise. Prefer fixing data over tuning knobs.

**The cost of teaching a small model to reach is still there**: irrelevance
41/44 to 36/44, and `W_ABSTAIN=2` no longer recovers it. That is why the
composite weights irrelevance at 0.40.

**`message_intent` is the wall.** 1 and 2 of 10 tool selections and 0 of 10 on
the arguments, from a base of 0. Classifying a quoted message and pulling a date
out of it is harder than filling three closed fields, and 58 slot rows across
both tools is not enough for the second.

**Slot filling moved for the first time, and data is what moved it.**
`search_filter` goes 0/17 to 16/17 on the tool and 0/17 to 3/17 on the arguments.
Three of seventeen is not a good number; it is the first one above zero.

## The run

```bash
pip install torch --index-url https://download.pytorch.org/whl/cu124
pip install transformers accelerate
EPOCHS=3 BS=4 python3 training/distil.py <base-hf-dir> <distil-dir> <out-dir>
# or, to compose the set as the third column above:
# EPOCHS=3 BS=4 W_ABSTAIN=2 W_SLOT=3 CAP_ANSWER=1 python3 training/distil.py ...
python3 llama.cpp/convert_hf_to_gguf.py <out-dir> --outfile model.gguf --outtype f16
cp <base-hf-dir>/tokenizer.json <model-dir>/       # the GGUF carries none Tacet can read
tacet bench run /somewhere/split/test/ALL.json --model <model-dir-name> --skip-missing
```

**Install llama.cpp's requirements BEFORE the CUDA torch, or not at all.** Its
`requirements.txt` pulls the CPU wheel over the top and training dies with "Torch
not compiled with CUDA enabled" — measured, and it cost a run.

**`TrainingArguments` drops keywords between transformers versions.**
`warmup_ratio` is gone from the one on the box this was last run on, and the
`TypeError` killed a run *after* the dataset had been generated and the GPU
hours paid for. `distil.py` now filters its arguments against the actual
signature and prints what it dropped. Pinning a version would only move the
failure to the next machine.
