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
# train 498 cases / test 165 cases
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
never a judge model. Measured: Qwen3-4B over the 498 train cases gave **764
pairs**, 750 of them usable.

## What is actually in the set, and how to count it wrong

The set is four different behaviours in one pile, and they are not the same
lesson:

| | | rows |
|---|---|---|
| `TOOL` | a call to an ordinary tool | 262 |
| `ANSWER` | the last step of a turn: a tool result is already in the prompt, and the right answer is prose | 304 |
| `ABSTAIN` | tools were offered and calling none was correct | 171 |
| `SLOT` | a call to `search_filter` or `message_intent`, where the arguments are the whole task | 13 |

**COUNTING `ABSTAIN` IS WHERE THIS GOES WRONG.** The obvious test — does the
prompt contain `<tool_response>` — matches **all 750 rows**, because the system
prompt names the tag while explaining it (*"If a `<tool_response>` block answers
what was asked…"*). It reports 0 abstentions where there are 171, and a balancing
pass built on that number oversamples answer-composition turns 4x while believing
it is teaching restraint, which is the exact opposite of the intended repair. A
tool result is only real when a **user turn opens with the tag**, so the marker
has to be the whole `<|im_start|>user\n<tool_response>`, which the system prompt
cannot contain.

| marker | ANSWER | ABSTAIN | TOOL | SLOT |
|---|---|---|---|---|
| `<tool_response>` | 475 | **0** | 262 | 13 |
| user turn + the tag | 304 | **171** | 262 | 13 |

**`SLOT` is 13 rows and no weighting fixes that.** The teacher passes 13 of the
27 argument-extraction cases in the train half, so tripling them is 39. Slot
filling is a data problem — more cases have to be written — and pretending
otherwise by oversampling thirteen rows until the score moves is how a benchmark
stops meaning anything.

## The one constraint that is easy to miss

**The prompt is stored RENDERED, so teacher and student must share a chat
template.** That is what makes the set usable as-is, and also its limit: a Gemma
student trained on ChatML prompts would be trained on a format Tacet never shows
it. Qwen3 and SmolLM2 are both ChatML, which is why the run below pairs them.

SmolLM2 converts to a GGUF whose architecture is `llama`, which Tacet loads —
and refuses if the tokenizer carries no `<|im_start|>`, because `llama` names a
family that does not agree on a template.

## What it bought, on cases the student never saw

165 held-out cases, RTX 3090, 5 Sep 2026. Training is 164 s unweighted and 300 s
composed.

| SmolLM2-135M | base | defaults | `W_ABSTAIN=2 W_SLOT=3 CAP_ANSWER=1` |
|---|---|---|---|
| composite | 47.7 | 54.9 | **62.4** |
| irrelevance | **36/38** | 28/38 | 34/38 |
| tool selection | 6/101 | 39/101 | 39/101 |
| correct call | 0.0% | 36.4% | 36.4% |
| `search_filter` tool / answer | 0/5 · 0/5 | 1/5 · 0/5 | **4/5** · 1/5 |
| `message_intent` | 0/4 | 0/4 | 0/4 |
| decode | 132 tok/s | 130–134 | 131–134 |
| peak resident | 528 MiB | 528 MiB | 528 MiB |
| training | — | 218 s | 300 s |

**Composing the set is worth more than the training run.** Same pairs, same
optimiser, same three epochs: pouring all 750 rows in gives 54.9 and loses eight
irrelevance cases; composing them gives 62.4, loses four, and takes
`search_filter` from 1/5 to 4/5.

**The weights are a dial between reaching and restraint.** A run that capped the
answer turns WITHOUT weighting the abstentions reached more tools — 48/101 — and
lost more of the gate, 27/38, for a composite of 57.9. There is no setting that
gives both; the composite is what decides the trade, and it weights irrelevance
at 0.40 for exactly this reason.

**Slot filling is the limit, and it is a data problem.** The student learns
*which* tool (`search_filter` 5/5) and not *what to put in it* (answer 1/5). No
weighting fixes 13 rows. More argument-extraction cases have to be written.

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
