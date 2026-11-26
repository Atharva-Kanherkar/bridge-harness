"""Score live_longmemeval output with LongMemEval's own judge prompts.

usage: python3 -I judge.py out.jsonl [more.jsonl ...]
Judge: `claude -p` on $LME_JUDGE_MODEL (default claude-sonnet-5-5), run in a
scratch directory. Verdicts are cached beside each input as <name>.judged.jsonl.
"""
import concurrent.futures, hashlib, json, os, subprocess, sys, tempfile
from collections import defaultdict

JUDGE_MODEL = os.environ.get("LME_JUDGE_MODEL", "claude-sonnet-5-5")
SCRATCH = os.environ.get("LME_JUDGE_CWD") or tempfile.mkdtemp(prefix="lme-judge-")

BASE = ("I will give you a question, a correct answer, and a response from a model. Please answer yes if the response "
        "contains the correct answer. Otherwise, answer no. If the response is equivalent to the correct answer or "
        "contains all the intermediate steps to get the correct answer, you should also answer yes. If the response "
        "only contains a subset of the information required by the answer, answer no. ")
TEMPLATES = {
    "single-session-user": BASE + "\n\nQuestion: {}\n\nCorrect Answer: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.",
    "single-session-assistant": BASE + "\n\nQuestion: {}\n\nCorrect Answer: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.",
    "multi-session": BASE + "\n\nQuestion: {}\n\nCorrect Answer: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.",
    "temporal-reasoning": BASE + "In addition, do not penalize off-by-one errors for the number of days. If the question asks for the number of days/weeks/months, etc., and the model makes off-by-one errors (e.g., predicting 19 days when the answer is 18), the model's response is still correct. \n\nQuestion: {}\n\nCorrect Answer: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.",
    "knowledge-update": "I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response contains some previous information along with an updated answer, the response should be considered as correct as long as the updated answer is the required answer.\n\nQuestion: {}\n\nCorrect Answer: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.",
    "single-session-preference": "I will give you a question, a rubric for desired personalized response, and a response from a model. Please answer yes if the response satisfies the desired response. Otherwise, answer no. The model does not need to reflect all the points in the rubric. The response is correct as long as it recalls and utilizes the user's personal information correctly.\n\nQuestion: {}\n\nRubric: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.",
}
ABSTAIN = "I will give you an unanswerable question, an explanation, and a response from a model. Please answer yes if the model correctly identifies the question as unanswerable. The model could say that the information is incomplete, or some other information is given but the asked information is not.\n\nQuestion: {}\n\nExplanation: {}\n\nModel Response: {}\n\nDoes the model correctly identify the question as unanswerable? Answer yes or no only."


def prompt_for(row):
    template = ABSTAIN if row["question_id"].endswith("_abs") else TEMPLATES[row["question_type"]]
    return template.format(row["question"], row["answer"], row["hypothesis"])


def judge(prompt):
    for _ in range(3):
        try:
            out = subprocess.run(["claude", "-p", "--model", JUDGE_MODEL], input=prompt, capture_output=True,
                                 text=True, cwd=SCRATCH, timeout=180).stdout.strip().lower()
            if out:
                return "yes" in out[:20]
        except subprocess.TimeoutExpired:
            pass
    return None


def score(path):
    rows = [json.loads(line) for line in open(path) if line.strip()]
    cache_path = path.replace(".jsonl", "") + ".judged.jsonl"
    cache = {}
    if os.path.exists(cache_path):
        for line in open(cache_path):
            item = json.loads(line)
            cache[item["key"]] = item["correct"]
    for row in rows:
        row["_key"] = hashlib.sha256((JUDGE_MODEL + prompt_for(row)).encode()).hexdigest()
    for _ in range(3):
        todo = [row for row in rows if row["_key"] not in cache and not row["error"]]
        if not todo:
            break
        with concurrent.futures.ThreadPoolExecutor(6) as pool, open(cache_path, "a") as out:
            for row, verdict in zip(todo, pool.map(lambda r: judge(prompt_for(r)), todo)):
                if verdict is not None:
                    cache[row["_key"]] = verdict
                    out.write(json.dumps({"key": row["_key"], "question_id": row["question_id"], "correct": verdict}) + "\n")
    by_type = defaultdict(list)
    for row in rows:
        row["correct"] = cache.get(row["_key"])
        if row["error"]:
            row["correct"] = False
        if row["correct"] is None:
            continue
        by_type[row["question_type"]].append(bool(row["correct"]))
    overall = [c for v in by_type.values() for c in v]
    summary = {
        "file": os.path.basename(path),
        "harness": rows[0]["harness"] if rows else None,
        "model": rows[0]["model"] if rows else None,
        "n": len(overall),
        "accuracy": round(100 * sum(overall) / max(1, len(overall)), 1),
        "task_avg": round(100 * sum(sum(v) / len(v) for v in by_type.values()) / max(1, len(by_type)), 1),
        "by_type": {k: f"{sum(v)}/{len(v)}" for k, v in sorted(by_type.items())},
        "errors": sum(1 for r in rows if r["error"]),
        "unjudged": sum(1 for r in rows if r["correct"] is None and not r["error"]),
        "mean_records": round(sum(r["active_records"] for r in rows) / max(1, len(rows)), 1),
        "mean_seconds": round(sum(r["seconds_extract"] + r["seconds_answer"] for r in rows) / max(1, len(rows)), 1),
        "mean_packet_chars": round(sum(r["packet_chars"] for r in rows) / max(1, len(rows))),
    }
    with open(path.replace(".jsonl", "") + ".scored.json", "w") as f:
        json.dump({"summary": summary, "rows": [{k: r[k] for k in ("question_id", "question_type", "question", "answer", "hypothesis", "correct", "active_records", "error")} for r in rows]}, f, indent=1)
    return summary


if __name__ == "__main__":
    for path in sys.argv[1:]:
        print(json.dumps(score(path)))
