# Managed agent runtimes

Bridge can install and remove the runtimes behind Claude, Codex, and OpenCode without owning the copies a user installed themselves. The integration code stays part of Bridge; the vendor runtime is the separately managed payload. Installation and authentication remain independent: Bridge never collects, proxies, migrates, or deletes a vendor credential.

The whole design turns on one question — *can Bridge prove it owns these bytes?* — and it is answered by a receipt, never inferred. Every layer re-asks it rather than trusting the layer below.

## Bridge is not a distributor

Payload bytes always come from the vendor's own source. What Bridge ships is the pinned version, the expected integrity, and the resolution logic. All three runtimes install as npm dependency closures, because npm is the official distribution channel for `@anthropic-ai/claude-agent-sdk`, `@openai/codex`, and `opencode-ai`, each publishes the same version there as on its releases page, and each ships its platform binary inside a platform-specific package.

Closures are pinned by lockfiles committed under `runtimes/`, so `npm ci` verifies every tarball against a recorded SRI hash, and installed with `--ignore-scripts`: the binary is already in the platform package, so no vendor postinstall needs to run and none does.

An npm tree is not byte-reproducible across machines — all three pull platform-specific binaries — so there is no honest constant to pin the installed tree against. The supply-chain guarantee is npm's per-tarball integrity from the lockfile; the tree digest described below is computed from the result and does a different job: proving ownership and catching later drift. The release-artifact source kind (a single published file with a publisher-pinned SHA-256) remains supported and tested for a future non-npm runtime.

npm creates `node_modules/.bin` shims as symlinks, and the payload engine rejects any tree containing a symlink, so those shim directories are pruned before the payload is digested. Bridge launches a platform binary by path and never resolves through `.bin`.

## Receipts and the integrity digest

An installation lives at a deterministic path derived from agent, version, platform, and integrity, and carries an embedded receipt plus an atomically written active receipt. Install copies into a sibling staging directory, verifies the digest of what was actually *stored* rather than what was promised, marks the entrypoint executable, then renames the complete installation into place. A crash before promotion leaves no active installation; a crash after promotion but before the active receipt leaves an embedded receipt that a retry of the identical recipe verifies and adopts without recopying.

The digest describes the logical tree that lands under `payload/`: entries sorted by canonical relative path, folded in as `dir\0<path>\0` or `file\0<path>\0<len><bytes>`. Three properties are deliberate. Paths are joined with `/` and must be valid UTF-8, so a tree digests identically on every host and two distinct non-UTF-8 names cannot collide. Directories are hashed, so an added or removed empty directory is drift rather than an invisible change. File bytes stream through the hasher, so memory stays flat regardless of payload size — a real runtime tree is a quarter of a gigabyte.

Permission bits are not hashed: they do not survive every transport, and hashing them would make a digest platform-specific. The one bit that matters is enforced instead — install marks the entrypoint executable, and status reports `EntrypointNotExecutable` when that stops being true, because an installation that can only fail with EACCES at spawn time is not ready.

Provenance is not identity. The receipt's `source` records where the installed bytes came from; `integrity_sha256` pins which bytes they are, and the installation id derives from agent, version, platform, and integrity without `source`. Comparing provenance in the ownership gate meant a relabelled but byte-identical artifact mapped onto the same directory and then failed its own check, wedging install and repair while status still reported healthy.

Uninstall reads and validates the receipt chain, requires every owned path to be a safe relative descendant of the managed root, and removes only that receipt-owned installation plus the active receipt. A corrupt or mismatched receipt, an unproven directory, a symlinked managed ancestor, or an owned path outside the exact installation all fail closed without deleting anything. Superseded installations are reclaimed on install and uninstall when their own receipt proves Bridge wrote them; a directory with no such proof is never pruned.

## Resolution order

Every agent resolves the runtime it will launch in one order: the executable the user configured explicitly, then a Bridge-managed receipt-bound payload, then a copy bundled with the app, then the system PATH. The first hit wins.

A runtime found on PATH resolves as external. It stays usable, it is reported so a user can see the copy they already have, and it is never Bridge's to remove. An explicit user configuration outranks even a managed payload, because overriding it would be Bridge deciding it knows better. A managed payload that has drifted is not handed out at all — resolution falls back rather than launching bytes that no longer match their receipt, so a tampered managed install degrades to the user's own copy instead of breaking the agent.

Managed payloads live under the leased data directory, registered by `BridgeCore::boot` rather than by each host so a future host cannot forget. Until a root is registered the managed tier is inert and the agents behave exactly as they did before managed payloads existed.

## Lifecycle states

Ten states — `not_installed`, `installing`, `installed`, `ready`, `running`, `stopping`, `uninstalling`, `external`, `broken`, `repairable` — with a closed transition relation. Payload ownership, integration readiness, and process liveness are three independent facts and are never collapsed into one boolean.

Two absences from the transition matrix are load-bearing rather than oversights. `running` has no edge to `uninstalling`, so a payload cannot be removed underneath a live process and a fresh launch cannot be handed a path being deleted; the only route is `running → stopping → uninstalling`. And no state Bridge does not own the payload in — `external` included — can begin uninstalling at all.

Only the in-flight states are stored. Every other state is derived from a fresh observation, because storing the last state and converging along legal edges would force the machine to lie: a payload that vanished out of band would have to be reported as having passed through `uninstalling`, which says Bridge removed it.

Crash loops are bounded. Consecutive launch or run failures count against a budget that saturates, so a long-lived agent cannot wrap the counter back into a state where Bridge would resume relaunching it. An agent whose budget is spent reports `broken` rather than `ready` — reporting readiness for something Bridge will refuse to start invites a caller to keep asking — while remaining reinstallable and removable. Retained failure context is redacted through `secret_interception::sanitize` and length-bounded, because the most useful context available is a provider's stderr tail, which is also the likeliest place for a token to appear.

## The agents RPC domain

`agents/list_managed_agents`, `inspect_managed_agent`, `install_managed_agent`, `repair_managed_agent`, and `uninstall_managed_agent`. Deliberately separate from the `marketplace` domain, which is about plugins running *inside* an agent: different lifecycle, different ownership, different failure modes.

Operations run to completion and their result says what happened — installed, repaired, removed, already current, already absent — plus the post-operation status, so a client needs no second round trip. There is deliberately no operation id and no progress stream: handing back a correlation id for a stream that will never reference it invites a client to wait forever. Backgrounding these is a future change with a new result shape, not a reinterpretation of this one.

Eight conditions get stable codes in a `3000` range: unsupported platform, integrity failure, external-not-managed, busy, corrupt receipt, vendor prerequisite missing, uninstall not permitted, and unknown agent. They live in a typed domain error mapped at the dispatch boundary rather than widening `BridgeError`, whose `1000` codes mirror its own variants; an underlying I/O or database failure keeps its own code instead of being flattened into a managed-agent condition. Both host modes deliver the same `{code, kind, message}` envelope, because a code dropped in one mode pushes clients straight back to matching message text.

Removal additionally refuses while a provider process is alive for that agent, checked against tracked adapter pids and their recorded OS identity. A stale row must not make an agent permanently un-removable, so identity is verified rather than assumed.

## The desktop surface

Settings → Harnesses → Agent runtimes. One card per integration, above harness configuration, because whether an agent is installed at all comes before how it behaves once it runs.

Removal is offered if and only if `status.removable` is true — the API's own answer to "is this Bridge's to remove". The UI never derives removability from a state string, so a future state it has never heard of cannot open the destructive path. A working user install reads as settled: it says it works, names itself as the user's own, and offers letting Bridge manage its own copy as a quiet opt-in rather than an instruction. Presenting that option as the card's only button made an agent the user could already chat with look like it needed installing.

Removing asks first, and the confirmation names the exact payload — label, version, path — and states what survives: conversation history, vendor configuration, sign-in, and any copy the user installed themselves. Focus lands on the keep action, which is also first in tab order, because a dialog that autofocuses its destructive action turns a stray Enter into an uninstall.

## Authentication boundary

Nothing here owns a credential. There is no API-key form, OAuth client, credential vault, login dashboard, or logout action, and no field in the agents wire surface concerns authentication. A vendor that needs a login surfaces its own message verbatim under the vendor-prerequisite code, and the agent stays `installed` rather than being recast as `broken` — nothing is broken, the user simply has not authenticated with the vendor.

## What is deliberately not done yet

Install and repair are synchronous. Measured against the live registries they take 15–32 seconds per agent, which is tolerable but blocks the caller; a background job runner with a real operation id and a progress stream is the follow-on, and the notification contract for it already exists.

`processId`, `consecutiveFailures`, `lastFailure`, and `vendorMessage` are declared on the status payload but not yet populated: they need the lifecycle coordinator wired with a readiness probe and a process supervisor, which is where those facts live. The UI renders them already.

Status recomputes the payload digest on every call with no mtime or size fast path. For claude's 5552-file tree that is measurable, and a cache needs an invalidation design rather than a quick fix.

The module's symlink defenses are check-then-use rather than capability-based, and `sync_directory` is a no-op off Unix, so the durability half of atomic promotion is weaker on Windows. Both are recorded rather than implied to be stronger than they are; migrating onto `cap-std`, already used by `workspace_files`, is a scoped follow-up.

## Verification

The per-behaviour tests run offline against fixtures. Because fixtures always have the shape their author imagined, a separate opt-in test drives the real thing:

```
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core \
  --test managed_agents_live -- --ignored --nocapture --test-threads=1
```

It installs each agent from its vendor registry, asserts the receipt's entrypoint is a real executable at the platform-package path, asserts no symlink survived into an owned payload, checks the digest is stable across reads, confirms a repeat install converges instead of redoing the work, uninstalls, confirms a user-owned runtime is byte-identical afterwards, and reinstalls. A second test proves removal is refused with the busy code while a live pid is recorded for the agent, against a real installed tree.

Measured on macOS arm64: claude 254 MB across 5552 files in 31.9s, codex 262 MB across 14 files in 28.3s, opencode 136 MB across 10 files in 14.8s. Two very different tree shapes exercising the same digest and pruning path is stronger evidence than either alone.

`scripts/check-builtin-adapters.sh` remains the gate that the three integrations' capabilities, sandbox modes, and normalization did not drift while any of this landed.
