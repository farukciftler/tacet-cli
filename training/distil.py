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
"""
import glob, json, os, sys, torch
from transformers import AutoModelForCausalLM, AutoTokenizer, TrainingArguments, Trainer

BASE = sys.argv[1]
DATA = sys.argv[2]
OUT = sys.argv[3]
EPOCHS = float(os.environ.get("EPOCHS", "3"))
MAXLEN = int(os.environ.get("MAXLEN", "2048"))

rows = []
seen = set()
for f in sorted(glob.glob(os.path.join(DATA, "*.jsonl"))):
    for line in open(f):
        r = json.loads(line)
        key = (r["prompt"], r["completion"])
        if key in seen:      # the same case can pass on several runs
            continue
        seen.add(key)
        rows.append(r)
print(f"pairs: {len(rows)}", flush=True)

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

args = TrainingArguments(
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
Trainer(model=model, args=args, train_dataset=data, data_collator=collate).train()
model.config.use_cache = True
model.save_pretrained(OUT)
tok.save_pretrained(OUT)
print("SAVED", OUT, flush=True)
