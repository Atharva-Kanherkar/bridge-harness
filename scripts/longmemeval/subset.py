"""Pick a stratified, seeded LongMemEval subset: N questions per question type.

usage: python3 -I subset.py longmemeval_oracle.json 10 > ids.txt
"""
import json, random, sys
from collections import defaultdict

data = json.load(open(sys.argv[1]))
per_type = int(sys.argv[2])
seed = int(sys.argv[3]) if len(sys.argv) > 3 else 0
groups = defaultdict(list)
for q in data:
    groups[q["question_type"]].append(q["question_id"])
rng = random.Random(seed)
for kind in sorted(groups):
    ids = sorted(groups[kind])
    rng.shuffle(ids)
    for qid in ids[:per_type]:
        print(qid)
