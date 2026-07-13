# Compaction and resume

Bridge distinguishes stored history, provider-native context, and projected restoration context. Persisting an entry does not mean it is injected into a provider thread.

## Restoration modes

| Mode | Meaning |
| --- | --- |
| `hot` | The adapter process is still running. |
| `native` | The provider can resume its stored thread or session ID. |
| `checkpoint_restored` | Native resume is unavailable or failed, so Bridge starts fresh with a validated projection. |
| `fresh` | The task intentionally starts without prior conversation context. |

Native capability is discovered from the installed harness. A failed native resume is recorded before checkpoint fallback; Bridge never labels a fresh process as natively resumed.

## Compaction ownership

Rust's compaction controller decides when a checkpoint is needed. The session being compacted authors its own semantic checkpoint because it owns its decisions, risks, and incomplete work. The orchestrator receives typed worker results and checkpoints, not raw worker transcripts.

Compaction triggers at context pressure, phase boundaries, before suspension or model downgrade, before risky native resume, or on manual request. It is suppressed during tool calls and approvals, for short-lived workers with valid final results, and when no meaningful work occurred since the last boundary.

Checkpoints and compaction boundaries are immutable forest entries. Original events remain stored. Validation failure gets one same-session repair; a dead process may use a recovery worker, whose result is explicitly marked reconstructed.

Schema validity is not enough for an agent-authored checkpoint. Before committing a boundary, Rust independently scans the active SQLite branch since the previous completed compaction, deduplicates durable worker decisions and file evidence, and requires the checkpoint to contain every item. An incomplete but well-formed response uses the same single repair allowance and then fails closed. Successful verification and its evidence counts are committed atomically as a `checkpoint.evidence_verified` audit event; reconstructed checkpoints continue to identify their independent provenance explicitly.
