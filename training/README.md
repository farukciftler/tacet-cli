# Distilling a tool-caller

A 135M model cannot call tools in Tacet's format. After five minutes of training
on a set Tacet generated from its own benchmarks, it can — 0% correct calls to
61.5%, at 127 tok/s and 528 MiB.

This directory is the recipe. It is deliberately small and deliberately outside
the Rust workspace: training needs PyTorch and a GPU, which this project does not
take a dependency on, and `cargo test` must never try to run any of it.

## Where the data comes from

Not from a hand-written dataset, and not from a bigger model's raw output —
most of which is wrong. It comes from **the subset of a teacher's output that a
benchmark scored as correct**:

```bash
TACET_DISTIL_DIR=/somewhere/distil \
TACET_WEB_CASSETTE=$PWD/benchmarks/cassettes \
  tacet bench run benchmarks/en/arithmetic-time.json --model qwen3-4b --json --skip-missing
```

Every step that PASSES writes `{"case", "prompt", "completion"}` as JSONL. A step
that called the wrong tool, or called nothing, contributes nothing — its prompt
is exactly the input where the student must *not* copy the teacher. "Correct" is
the benchmark's own pass/fail, never a judge model.

Run it over every benchmark file and the union is the set. Measured: Qwen3-4B
over 665 cases produced **1031 pairs**, 1017 of them usable — `create_document`
70, `calculate` 67, `read_document` 47, `checksum` 36, `find_file` 35, and a long
tail.

## The one constraint that is easy to miss

**The prompt is stored RENDERED, so teacher and student must share a chat
template.** That is what makes the set usable as-is, and it is also its limit: a
Gemma student trained on ChatML prompts would be trained on a format Tacet never
shows it. Qwen3 and SmolLM2 are both ChatML, which is why the run below pairs
them.

## The run

```bash
pip install torch --index-url https://download.pytorch.org/whl/cu124
pip install transformers accelerate
EPOCHS=3 BS=4 python3 training/distil.py <base-hf-dir> <distil-dir> <out-dir>
python3 llama.cpp/convert_hf_to_gguf.py <out-dir> --outfile model.gguf --outtype f16
cp <base-hf-dir>/tokenizer.json <model-dir>/       # the GGUF carries none Tacet can read
tacet bench gap benchmarks/en/arithmetic-time.json --model <model-dir-name>
```

**Install llama.cpp's requirements BEFORE the CUDA torch, or not at all.** Its
`requirements.txt` pulls the CPU wheel over the top and training dies with "Torch
not compiled with CUDA enabled" — measured, and it cost a run.

## What it bought, and what it cost

SmolLM2-135M-Instruct, RTX 3090, 3 epochs in 310 seconds:

| | before | after |
|---|---|---|
| started a call | 2.6% | **84.6%** |
| valid **if** started | 100% | 100% |
| **correct call** | **0.0%** | **61.5%** |
| decode | 127 tok/s | 127–134 tok/s |
| peak resident | 529 MiB | 528 MiB |

`search_filter` benchmark: **44.0 → 76.4 / 100**, tool selection 0/16 → 14/16.

**And the cost, which is the part worth reading twice: the irrelevance gate went
from 4/4 to 3/4.** Teaching a model to reach for tools makes it likelier to reach
for one when it should not. That is why the composite weights irrelevance
heaviest — so a gain in tool accuracy cannot quietly buy a loss in restraint.

Slot filling barely moved (0/16 → 5/16): the student learned *which* tool, not
*what to put in it*. That is the next thing to train on, and it is visible only
because the task benchmarks score the arguments separately.
