# Native dictation evaluation

Status: development spike, not a shipped provider or a final model selection.
This follows Part 2 of [the approved plan](voice-dictation-rework.md).

## Candidate and integration boundary

Evaluate sherpa-onnx **1.13.8**, CPU, with the same English Nemotron 560 ms int8
model that Omnigent uses. Run through its native C ABI, without Python, a coding
session, authentication, or microphone access. This tests the inference boundary
that the upstream Rust wrapper also uses; it does not test Rust ownership,
daemon integration, endpoint segmentation, WKWebView capture, or packaging.

Upstream provides [Rust bindings and linking options](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.8/sherpa-onnx/rust/sherpa-onnx/src/lib.rs)
and a [Nemotron C example](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.8/c-api-examples/streaming-nemotron-c-api.c).
The Rust build script can download native libraries automatically. Bridge's
eventual build must supply preverified libraries explicitly, not acquire an
untracked binary during an ordinary Cargo build. Keep model acquisition a
separate, explicit application action.

A supervised native helper is the preferred integration boundary to investigate:
native crashes and noninterruptible inference should not take down `bridged` or
retain its worker indefinitely. The daemon will own the provider selection, one
active take, bounded PCM input, deadlines, and opaque take IDs. The helper must
retain its model across warm takes, and be terminable on timeout. This is an
architectural recommendation, not yet an implemented helper.

### License correction

Omnigent's [download script](https://github.com/omnigent-ai/omnigent/blob/e047da5f734e0db77a6d4c32abb3dce433a2e216/scripts/fetch-dictation-models.sh)
describes its models as Apache-2.0. NVIDIA's [authoritative model card](https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b)
instead identifies the **NVIDIA Open Model License** for the ASR weights. Do not
copy the script's license claim into Bridge. Review and preserve the exact
[model license](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/)
and notices before redistribution; separately retain the inference runtime's
and its dependencies' notices. No model or native library is committed here.

## Provisional evaluation gates

These are engineering targets set before the first run, not measured results or
user-approved release guarantees. Run the broader accuracy suite and baseline
supported-Mac matrix before a production choice:

- Compressed model download <=600 MiB; peak helper RSS <=1.5 GiB.
- Cold process model initialization <=5 seconds (OS file-cache state recorded).
- Every preliminary take decodes at real-time factor <=0.5, including final flush.
- Final flush <=1 second, without dropped tail words.
- Silence produces no text; repeated streams remain isolated.
- Release corpus: >=30 consented prose/developer utterances, <=10% normalized
  prose WER, >=90% exact identifier/path recovery, and no missing final words in
  short-tail fixtures. Noise and accents must be represented separately.
- English only for this candidate; Hindi/Hinglish remains unevaluated. No inferred
  multilingual support from the runtime's ability to load different models.

## Reproduce the native spike

Use an isolated temporary directory. Download these exact artifacts from the
[upstream release](https://github.com/k2-fsa/sherpa-onnx/releases/tag/v1.13.8)
and [ASR model release](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models),
verify SHA-256 **before extraction/execution**, and inspect archive paths:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `sherpa-onnx-v1.13.8-osx-arm64-shared-no-tts.tar.bz2` | 18,252,168 | `91b96512c4fa1960f8a9ed5360a6c8dda53a4b5015d0590244f14086a234557a` |
| `sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25.tar.bz2` | 463,945,051 | `78e2b79fcf7271553a74402a76b771b09ea40117a39566a79f52235b23db6358` |

The model archive supplies public `test_wavs/0.wav`, `1.wav`, and `trans.txt`.
Do not add these recordings to the repository or describe them as a consented
Bridge-user evaluation corpus. `8k.wav` is intentionally outside the benchmark's
16 kHz input contract and should be rejected.

From the repository root, set `VOICE_RUNTIME_DIR` and `VOICE_MODEL_DIR` to the
verified, extracted directories, and `VOICE_EVAL_DIR` to your temporary directory:

```sh
clang -std=c11 -O2 -Wall -Wextra -Werror \
  -I "$VOICE_RUNTIME_DIR/include" scripts/benchmark-dictation-native.c \
  -L "$VOICE_RUNTIME_DIR/lib" -lsherpa-onnx-c-api \
  -Wl,-rpath,"$VOICE_RUNTIME_DIR/lib" \
  -o "$VOICE_EVAL_DIR/benchmark-dictation"

"$VOICE_EVAL_DIR/benchmark-dictation" "$VOICE_MODEL_DIR" 4 \
  "$VOICE_MODEL_DIR/test_wavs/0.wav" "$VOICE_MODEL_DIR/test_wavs/1.wav" --silence
```

Each fixture runs three times in new streams sharing one recognizer. Repeat in
a new process with one CPU thread to compare against four. The benchmark accepts
only bounded 16 kHz WAV input or its own ten-second silence fixture. It opens no
microphone or network connection and installs nothing in Bridge. JSON-lines
output excludes transcript text and input paths by default; `--print-text` before
the fixture arguments opts in for public/consented accuracy inspection only.

Input arrives in 100 ms blocks without wall-clock pacing. `decode_ms` and
`real_time_factor` measure throughput, **not end-to-end dictation latency**.
`first_partial_input_ms` is the amount of audio consumed before a nonempty result,
not elapsed wall time. Finalization includes one second of synthetic silence,
matching Omnigent's tail handling; this silence is excluded from audio duration.
Peak RSS is process-wide and cumulative, not per-take incremental memory. Model
initialization is cold-process, not guaranteed cold-disk-cache.

## Observations — 2026-09-21

Host: Apple M4, 16 GiB RAM, macOS 27.0 build 26A428, arm64. No microphone,
coding session, account credential, or network transcription was used. Artifact
SHA-256 values above matched before extraction. Public fixtures were 6.625 and
16.715 seconds long; each ran three times, followed by three ten-second silence
takes. The two thread-count runs were sequential, not concurrent.

| Metric | 1 CPU thread | 4 CPU threads |
| --- | ---: | ---: |
| Process model load | 1.232 s | 1.394 s |
| Speech real-time factor, six takes | 0.206–0.253 | 0.132–0.165 |
| Speech CPU time, 16.715-second fixture | 3.316–3.654 s | 9.918–10.286 s |
| Final flush, all nine takes | 205–305 ms | 125–179 ms |
| First nonempty partial, audio consumed | 1.300 s | 1.300 s |
| Peak process RSS | 1,115 MiB | 1,123 MiB |
| Silence text, three takes | Empty | Empty |

The four-thread transcripts matched both supplied references after removing
punctuation and case (0 word edits over 66 reference words, repeated three times).
The one-thread run intentionally omitted transcript text from output; identical
byte counts do not establish identical words. These two literary samples provide
no evidence for code identifiers, paths, accents, noise, or Hindi/Hinglish.
No missing last word was observed in these samples, but short-tail and live
capture remain untested. Repeated streams did not leak text into subsequent
silence streams. This small run is not a percentile latency study or soak test.

The extracted model directory occupies approximately 632 MiB; shared libraries
occupy 30 MiB. Compressed model download is approximately 442.5 MiB. The C API
library declares macOS 11.0 as its minimum; Bridge declares 12.0. This metadata
does not substitute for testing the complete dependency chain on macOS 12 or an
Intel Mac. The runtime and model were downloaded only into a temporary evaluation
directory, not installed into Bridge or added to the repository.

Validation: C11 compilation passed with `-Wall -Wextra -Werror`; all 18 native
takes completed; the 8 kHz fixture was rejected with exit status 1; five invalid
argument cases returned exit status 2. Default output contains neither fixture
paths nor transcript fields. `git diff --check` passed. Production frontend and
core code were not changed by this spike. The 58 existing focused voice controller,
capture, hook, and composer tests were rerun and passed. After recording these
results, the temporary benchmark binary, extracted runtime/model, and downloaded
archives were removed (approximately 1.1 GiB); they can be rebuilt/redownloaded
using the pins above. The benchmark source and this report remain in the checkout.

### Decision and remaining gates

**Proceed with sherpa-onnx as the integration candidate**, starting at one CPU
thread: it cleared the preliminary throughput/flush budget while using much less
aggregate CPU than four threads. Four threads were faster in wall time but are
not automatically the better default beside active coding agents. Keep thread
count configurable internally for baseline-device evaluation.

**Do not finalize the model or advertise local dictation as ready yet.** The
working set is substantial, and the broader accuracy corpus, baseline Macs,
model-license notices, helper supervision, model setup/removal, and live capture
are still open. No runtime dependency, auto-download, provider fallback, or
shipped microphone behavior was changed by this spike. Next is the independent
daemon service and deterministic fake-provider contract, then the supervised
native helper behind it. Keep the remaining Codex lifecycle release blockers in
the original plan visible.
