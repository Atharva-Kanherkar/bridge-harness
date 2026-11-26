# Send latency test contract

The benchmark drives the composer’s core API calls against a generated database with 1,500 visible chats, 50,000 background runs, and 300,000 forest entries. The workspace is a committed Git checkout. The adapter accepts input in process and supplies deterministic completion frames. Each run starts with fresh data, and builds, fixture creation, and fake turn completion happen outside the send timers.

| Operation | Correctness check | p95 target |
| --- | --- | ---: |
| Create, start, and submit | The first message reaches the adapter, returns `startedNewTurn`, and is saved in the session forest. | 300 ms |
| Prepare and submit a follow-up | The idle runtime accepts and persists the message without another launch. | 50 ms |
| Persist a provider frame | One frame commits one forest entry and advances its session head. | 5 ms |
| Read and serialize app state | Visible chats are returned and background runs stay out of the result. | 150 ms |

The compatibility state endpoint still includes background runs. Create and start keep their existing return shapes until the composer adopts the newer entry points from [#1040](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1040). The existing repository stamp tests cover streaming writes. The general guard against process spawns depends on [#1042](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1042) and [#1043](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1043).

The comparator accepts three matched rounds per revision. It checks the median of each hop’s round p95s and reports absolute targets alongside the relative result. The comparison fails on missing samples or incompatible fixture versions. A single noisy round must leave an otherwise stable result green. A sustained 50 ms delay in persistence must turn it red.
