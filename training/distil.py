"""Distil a small ChatML model on the teacher's CORRECT turns.

The dataset is what `TACET_DISTIL_DIR` wrote: one JSON object per line with the
RENDERED prompt (ChatML, ending in the assistant turn) and the completion the
teacher produced on a step the benchmark scored as passing.

TEACHER AND STUDENT MUST SHARE A TEMPLATE. The prompt is stored rendered, which
is what makes it usable as-is — and also what limits it: a Gemma student would be
trained on a format Tacet never shows it. SmolLM2 is llama-arch ChatML, the same
rendering Qwen3 produces, so the set transfers unchanged.

LOSS ON THE COMPLETION ONLY. Training on the prompt too teaches the model to
generate tool catalogues, which is the one thing it will never be asked for.

GENERATE THE SET FROM A TRAIN SPLIT, NOT FROM EVERY BENCHMARK FILE. `split.py`
next to this script writes one; the first run of this recipe did not use it and
reported the student's score on cases whose own answers were in its training
set. See training/README.md.
"""
import glob, inspect, json, os, sys, torch
from transformers import AutoModelForCausalLM, AutoTokenizer, TrainingArguments, Trainer

BASE = sys.argv[1]
DATA = sys.argv[2]
OUT = sys.argv[3]
EPOCHS = float(os.environ.get("EPOCHS", "3"))
MAXLEN = int(os.environ.get("MAXLEN", "2048"))

W_ABSTAIN = int(os.environ.get("W_ABSTAIN", "1"))
W_SLOT = int(os.environ.get("W_SLOT", "1"))
CAP_ANSWER = os.environ.get("CAP_ANSWER") == "1"

# A USER TURN THAT OPENS WITH THE TAG. The system prompt also names
# `<tool_response>` while explaining it, so a bare substring test calls every
# single row an answer-composition turn and reports zero abstentions where there
# are 171. See README.md.
RESULT_IN_PROMPT = "<|im_start|>user\n<tool_response>"
SLOT_TOOLS = ("search_filter(", "message_intent(")

rows = []
seen = set()
for f in sorted(glob.glob(os.path.join(DATA, "*.jsonl"))):
    for line in open(f):
        r = json.loads(line)
        key = (r["prompt"], r["completion"])
        if key in seen:      # the same case can pass on several runs
            continue
        seen.add(key)
        # `(36)` and `840` are the teacher echoing the tool result back as the
        # whole answer. Each one teaches the student to answer with a bare number.
        if len(r["completion"].strip()) < 8:
            continue
        rows.append(r)
print(f"pairs: {len(rows)}", flush=True)

def group(r):
    """Which of the four behaviours this row teaches."""
    c = r["completion"].strip()
    is_call = "(" in c[:40] and c[: c.find("(")].replace("_", "").isalnum()
    if not is_call:
        return "ANSWER" if RESULT_IN_PROMPT in r["prompt"] else "ABSTAIN"
    return "SLOT" if any(t in c for t in SLOT_TOOLS) else "TOOL"

# COMPOSING THE SET IS OFF BY DEFAULT, so this script's plain run is the number
# the README quotes for it. MEASURED on the held-out half, same pairs, same
# optimiser, same three epochs: all 750 rows poured in gives a composite of 54.9
# and irrelevance 28/38; `W_ABSTAIN=2 W_SLOT=3 CAP_ANSWER=1` gives 62.4 and
# 34/38, and takes `search_filter` from 1/5 to 4/5. Capping the answers WITHOUT
# weighting the abstentions goes the other way — 48/101 tools, 27/38 gate — so
# these are a dial between reaching and restraint rather than an improvement.
# Oversampling by repetition keeps the loss and the optimiser exactly as they
# are, so the comparison measures the SET and nothing else.
if W_ABSTAIN > 1 or W_SLOT > 1 or CAP_ANSWER:
    import random
    buckets = {k: [] for k in ("ANSWER", "ABSTAIN", "SLOT", "TOOL")}
    for r in rows:
        buckets[group(r)].append(r)
    print("groups:", {k: len(v) for k, v in buckets.items()}, flush=True)
    rng = random.Random(0)
    answers = buckets["ANSWER"]
    if CAP_ANSWER:
        answers = rng.sample(answers, min(len(answers), len(buckets["TOOL"])))
    rows = (buckets["TOOL"] + buckets["SLOT"] * W_SLOT
            + buckets["ABSTAIN"] * W_ABSTAIN + answers)
    rng.shuffle(rows)
    print(f"composed: {len(rows)}", flush=True)

tok = AutoTokenizer.from_pretrained(BASE)
if tok.pad_token is None:
    tok.pad_token = tok.eos_token

def encode(r):
    p = tok(r["prompt"], add_special_tokens=False)["input_ids"]
    c = tok(r["completion"] + "<|im_end|>", add_special_tokens=False)["input_ids"]
    ids = (p + c)[:MAXLEN]
    # -100 on the prompt: the loss is only about what the model must SAY.
    labels = ([-100] * len(p) + c)[:MAXLEN]
    return {"input_ids": ids, "labels": labels}

data = [encode(r) for r in rows]
data = [d for d in data if any(l != -100 for l in d["labels"])]
print(f"usable: {len(data)}", flush=True)

def collate(batch):
    n = max(len(b["input_ids"]) for b in batch)
    pad = tok.pad_token_id
    return {
        "input_ids": torch.tensor([b["input_ids"] + [pad] * (n - len(b["input_ids"])) for b in batch]),
        "labels": torch.tensor([b["labels"] + [-100] * (n - len(b["labels"])) for b in batch]),
        "attention_mask": torch.tensor([[1] * len(b["input_ids"]) + [0] * (n - len(b["input_ids"])) for b in batch]),
    }

model = AutoModelForCausalLM.from_pretrained(BASE, torch_dtype=torch.bfloat16).cuda()
model.gradient_checkpointing_enable()
model.config.use_cache = False

# FILTERED AGAINST THE SIGNATURE, not passed blind. TrainingArguments drops
# keywords between transformers versions — `warmup_ratio` is absent from the one
# on the box this was last run on, and the TypeError killed a run AFTER the
# dataset had been built and the GPU paid for. Pinning a version would only move
# the failure to the next machine; dropping what this install does not take, and
# saying so, does not.
wanted = dict(
    output_dir=OUT + "-ckpt",
    num_train_epochs=EPOCHS,
    per_device_train_batch_size=int(os.environ.get("BS", "4")),
    gradient_accumulation_steps=4,
    learning_rate=float(os.environ.get("LR", "1e-4")),
    lr_scheduler_type="cosine",
    warmup_ratio=0.05,
    logging_steps=10,
    save_strategy="no",
    bf16=True,
    report_to=[],
)
accepted = set(inspect.signature(TrainingArguments.__init__).parameters)
dropped = [k for k in wanted if k not in accepted]
if dropped:
    print("TrainingArguments does not accept:", dropped, flush=True)
args = TrainingArguments(**{k: v for k, v in wanted.items() if k in accepted})
Trainer(model=model, args=args, train_dataset=data, data_collator=collate).train()
model.config.use_cache = True
model.save_pretrained(OUT)
tok.save_pretrained(OUT)
print("SAVED", OUT, flush=True)
