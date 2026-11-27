# Eval: LongMemEval on Bridge memory

[LongMemEval](https://github.com/xiaowu0162/LongMemEval) (Wu et al., ICLR 2025)
asks 500 questions about a user's past chats: facts they said, things the
assistant said, preferences, changes over time, counts across chats, and
questions with no answer. This eval runs it through Bridge's real memory path
on every harness that attaches the `bridge` tool server, and scores it with the
benchmark's own judge prompts.

## What runs

`memory_longmemeval::live_longmemeval` (ignored, real provider turns). For each
question:

1. An empty `account:local` scope in a scratch database.
2. Each haystack session, oldest first, becomes a `direct` chat whose entries
   carry the session's recorded date.
3. Production extraction runs on it: `build_digest`, the extraction prompt, a
   hidden tool-free turn on the eval profile, `gate_and_insert`. Long chats are
   read in further runs from the watermark, as production does.
4. Every proposal is approved (a person who accepts everything in Review
   first), and the new records take the session's date.
5. The question is asked in a real chat on the same profile, prefixed with
   `(Today is <question date>.)`. Its session-context frame carries the memory
   guidance and the memory packet, and the `bridge` server offers
   `memory_search`, `memory_propose`, `chats_search` and `chats_read`, exactly as
   a top-level Bridge chat gets them. `LME_RECALL=ledger` deletes the haystack
   chats before asking, so only the ledger can answer.

`scripts/longmemeval/judge.py` grades each reply with the LongMemEval judge
prompt for its question type (`claude -p`, `claude-sonnet-5-5`; the paper used
GPT-4o) and reports accuracy per type.

## How to run

```bash
# once per worktree
bun install
cd src-tauri && cargo build -p bridged && cargo test -p bridge-core --lib --no-run

# data (MIT): https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned
python3 -I scripts/longmemeval/subset.py longmemeval_oracle.json 10 > ids60.txt

export BRIDGE_CLAUDE_SIDECAR=$PWD/sidecar/claude-agent/index.mjs
export BRIDGE_VISUALIZE_MCP_BIN=$PWD/src-tauri/target/debug/bridged
export LME_TEST_BIN=src-tauri/target/debug/deps/bridge_core-<hash>
export LME_RESULTS=/tmp/lme LME_CACHE=/tmp/lme/cache
scripts/longmemeval/run.sh claude claude claude-sonnet-5-5 longmemeval_oracle.json ids60.txt 4
scripts/longmemeval/run.sh codex codex gpt-6.1-sol longmemeval_oracle.json ids60.txt 4
scripts/longmemeval/run.sh opencode opencode openai/gpt-5.5 longmemeval_oracle.json ids60.txt 4
python3 -I scripts/longmemeval/judge.py /tmp/lme/*.jsonl
```

`LME_CACHE` keeps extraction answers keyed by profile, prompt and digest, so a
rerun that changes only retrieval or answering costs no extraction calls. A
rerun skips questions already in the output file.

OpenCode's default free model ran out of free usage during the first attempt,
so OpenCode runs on `openai/gpt-5.5`.

## Results

Subset: 60 questions from `longmemeval_oracle` (10 per question type, seed 0,
3 of them abstention). The oracle variant gives each question only its evidence
sessions (1 to 6 chats), so it measures extraction, retrieval and answering
without distractor chats.

### Baseline (main at `55a8db69`, first judge pass)

| Harness · model | Overall | single user | single assistant | preference | multi-session | temporal | knowledge update | records / Q | s / Q |
|---|---|---|---|---|---|---|---|---|---|
| Claude · claude-sonnet-5-5 | **65.0%** | 9/10 | 9/10 | 1/10 | 6/10 | 6/10 | 8/10 | 2.0 | 18 |
| OpenCode · openai/gpt-5.5 | **61.7%** | 8/10 | 9/10 | 2/10 | 5/10 | 5/10 | 8/10 | 3.6 | 23 |
| Codex · gpt-6.1-sol | **50.0%** | 7/10 | 8/10 | 0/10 | 7/10 | 2/10 | 6/10 | 1.6 | 34 |

What the misses have in common:

- **Extraction keeps almost nothing.** The prompt allows only preferences,
  conventions, decisions, constraints and identity facts, so "I bought a
  Fender Stratocaster", "I attended Emma's graduation" or "I stayed in a $30
  hostel in Tokyo" never reach the ledger: 1.6 to 3.6 records per question.
  Most correct answers came from `chats_search` over the raw chats.
- **Nothing carries a date.** The digest has no date and `memory_search`
  returns none, so "which came first" and "how many months ago" fail even when
  both facts are remembered. Codex answered "I remember both trips, but I don't
  have dates" on several.
- **`memory_search` needs every word.** It ANDs the query's tokens with no
  stemming, so "Rachel colleague" or "Italian restaurants" miss a record that
  says "restaurant". `chats_search` already relaxes to OR.
- **Advice questions never search.** Preference questions ("any tips for
  keeping my kitchen clean?") need what the person said before, but the
  guidance only tells the agent to search when asked what it remembers.
- Codex also called foreign memory tools (`recall_memories`,
  `recall_episodes`) from the user's own MCP config.

### After the fixes in #1190 (one judge pass over every run)

The judge treated a usage-limit message as "no" in the first pass, so every
run below was re-scored together with the hardened judge, which only accepts a
yes or a no.

| Harness · model | Before | After | temporal before / after | preference before / after |
|---|---|---|---|---|
| Claude · claude-sonnet-5-5 | 66.7% | **81.7%** | 5/10 · 7/10 | 2/10 · 6/10 |
| Codex · gpt-6.1-sol | 50.0% | **86.7%** | 2/10 · 8/10 | 0/10 · 7/10 |
| OpenCode · openai/gpt-5.5 | 58.3% | **83.3%** | 5/10 · 10/10 | 1/10 · 5/10 |

`longmemeval_s` (about 50 chats per question, mostly distractors), 30
questions (5 per type), after the fixes: Codex 80.0%, OpenCode 83.3%. The
Claude runs at this size hit the subscription's 5-hour limit and are being
rerun. With 60 questions, one question is 1.7 points, and two judge passes over
the same replies differed by up to 7 points, so small differences are noise.
