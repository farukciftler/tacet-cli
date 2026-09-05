"""The benchmark messages, one per line, for slots.c to read on stdin."""
import json, os, sys
repo = os.environ.get("TACET_REPO") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out = []
for name in ("search_filter", "message_intent"):
    for case in json.load(open(f"{repo}/benchmarks/tasks/{name}.json", encoding="utf-8"))["cases"]:
        for step in case["steps"]:
            out.append(step["message"])
sys.stdout.write("\n".join(out) + "\n")
