# Approved agent prompt changes

Bridge's adaptive learning ranks eligible harness/model/effort candidates. It
does not optimize prompt text. This feature supplies a separate, explicit path
for agents to request persistent guidance changes, with human review and an
immutable origin record. It does not run an automatic learning loop.

## Decisions from the research

1. **Edit Bridge guidance, not provider base prompts.** Add a dedicated
   `additional_guidance` section to the orchestrator and each worker role.
   It starts empty and contributes no compiled bytes until populated. The
   shipped role/delegation contracts remain dynamic, including worker depth.
2. **Append through proposals.** Agents submit additions and a rationale. The
   host calculates exact before/after text. Agents cannot delete or replace
   existing instructions through this tool. User editing and restoration remain
   available in Prompt Studio.
3. **Use existing role identity honestly.** These are shared role defaults,
   not per-configured-agent prompts. The current runtime selects worker prompts
   by role and does not persist a trustworthy configured-agent binding.
4. **Keep authorization in the host.** A new deterministic prompt-mutation
   decision authenticates actor/target from stored sessions and leases. Every
   accepted request becomes a human approval. Learning and provider permission
   bypass settings never authorize the write.
5. **Activate on the next matching launch.** Approval updates durable guidance.
   It does not interrupt running turns. Existing launch hash checks detect
   changed instructions when Bridge starts or relaunches that role.
6. **Reuse the established UI and host protocol.** A typed assistant control
   fence works across Bridge's adapters, as delegation already does. The
   conversation approval card shows the exact text; Prompt Studio shows history
   and restores revisions. Native SDK tool wrappers can be added separately.

## Request flow

```mermaid
sequenceDiagram
    participant Agent
    participant Host as Bridge runtime
    participant Policy as Mutation policy
    participant DB as Prompt store
    participant User
    Agent->>Host: bridge-prompt-change(requestId, targetSessionId?, guidance, rationale)
    Host->>Policy: Trusted actor session and turn
    Policy-->>Host: Require human approval or reject
    Host->>DB: Snapshot guidance state, revision and hash; persist proposal
    Host->>User: Exact before/after, origin, rationale, shared role scope
    User->>Host: Approve or decline this change
    Host->>Policy: Recheck role grant and target relationship
    Host->>DB: Compare snapshot; atomically append revision and resolve
    DB-->>Host: Applied, declined, stale or already resolved
    Host-->>Agent: Durable host outcome through normal lifecycle gates
```

The request is schema version 1. `targetSessionId` omitted means self. The
orchestrator may name one of its own worker sessions; Bridge resolves that
worker's role. Workers can request only their own role, and only when the role
is enabled under Settings → Permissions. Enabling proposals does not approve
any actual edit. Direct/hidden sessions have no mutation authority.

For example, an authorized orchestrator can propose for its own shared role:

````markdown
```bridge-prompt-change
{
  "schemaVersion": 1,
  "requestId": "verification-guidance-1",
  "guidance": "Name the checks that passed and any checks that could not run.",
  "rationale": "The previous completion report omitted verification evidence."
}
```
````

An orchestrator targeting a worker adds that worker's `targetSessionId`.
It does not supply a target role or actor identity. The host resolves both.
The assistant ends this control turn and waits for the host outcome before
continuing the original objective.

Approval details explicitly state that guidance affects every subsequent
matching role launch, including other workspaces using the same configuration.
An approval cannot be converted into permission for later changes.

## Durable correctness

Proposal replay uses actor session, turn and request id. A repeated identical
request returns its prior state; conflicting reuse fails. One proposal per
actor turn bounds flooding. Malformed/nested/quoted fences and model-supplied
actor identity cannot become host authority.

Acceptance compares current state, latest revision id and exact text hash to
the proposal snapshot. Revision identity catches an intervening edit that
restored identical text. The active prompt update, attributed append-only
revision, proposal settlement and approval resolution share one transaction.
Failures leave the previous prompt intact.

The origin records actor session, turn, role, proposal id and rationale. Old
revisions keep their existing data; restoring an old revision appends a new
human revision. Provider-owned base prompt capability claims stay unchanged.

## Implementation order and evidence

1. Lock the test contract and add guidance storage, migration, provenance and
   deterministic authority tests.
2. Connect authenticated runtime requests and durable human approval resolution,
   including worker waiting/result handling and stopped-session visibility.
3. Add exact-text approval UI, role opt-in controls and attributed history.
4. Regenerate protocol artifacts, validate both hosts, and run focused and full
   tests plus browser fixture verification.

The executable acceptance cases are in
[the test contract](../../testing/feat-agent-prompt-mutation.md).

## Primary references

- [Claude system prompt customization](https://code.claude.com/docs/en/agent-sdk/modifying-system-prompts)
  distinguishes provider presets from appended application instructions.
- [Claude permission controls](https://code.claude.com/docs/en/agent-sdk/permissions)
  describe provider-tool authorization; Bridge configuration has its own gate.
- [SQLite transactions](https://www.sqlite.org/lang_transaction.html) establish
  the transaction boundary used for snapshot comparison and atomic persistence.
