#!/bin/sh
# usage: run.sh <label> <harness> <model|-> <data.json> <ids.txt> <shards> [recall]
# Runs live_longmemeval in <shards> parallel processes into $LME_RESULTS/<label>.jsonl.
set -eu
label=$1 harness=$2 model=$3 data=$4 ids=$5 shards=$6 recall=${7:-full}
: "${LME_RESULTS:?}" "${LME_TEST_BIN:?}" "${BRIDGE_VISUALIZE_MCP_BIN:?}"
[ "$model" = "-" ] && model=""
mkdir -p "$LME_RESULTS/logs"
i=0
while [ $i -lt "$shards" ]; do
  BRIDGE_MEMORY_EVAL_HARNESS=$harness BRIDGE_MEMORY_EVAL_MODEL=$model LME_DATA=$data LME_IDS=$ids \
  LME_OUT="$LME_RESULTS/$label.s$i.jsonl" LME_SHARD="$i/$shards" LME_RECALL=$recall \
    "$LME_TEST_BIN" memory_longmemeval --ignored --nocapture > "$LME_RESULTS/logs/$label.s$i.log" 2>&1 &
  i=$((i + 1))
done
wait
cat "$LME_RESULTS/$label".s*.jsonl > "$LME_RESULTS/$label.jsonl"
