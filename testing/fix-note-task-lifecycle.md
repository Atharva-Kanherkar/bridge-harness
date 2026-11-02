# fix/note-task-lifecycle Test Contract

## Functional Behavior
- Append work to a note and rerun Turn into tasks: add only new work. The extractor must explicitly identify an existing task by its supplied id when rephrasing already tracked work. Source quotations alone must never suppress a distinct new outcome, including one sharing an existing sentence. Missing or unknown identity decisions on reruns fail without creating tasks.
- Keep existing task ids, edited titles and bodies, states, and linked chats, including done and archived tasks. Suppress repeated excerpt/title pairs within one reply. Separate outcomes may share a source sentence and must all survive their first extraction; distinct excerpts remain separate tasks.
- Give extraction the existing task titles and excerpts so it can avoid proposing work already tracked.
- Linked task rows show the latest chat's current status and Open chat, without Start. Starting, running, waiting, finished-turn, failed and stopped states are distinct. Marking done and reopening remain explicit user actions.
- Unstarted open tasks retain Start. Done and archived tasks do not invite a new start. A linked task's own page labels an intentional additional chat as New chat.
- Repeated clicks while a task is starting create at most one chat. A failed start can be retried.

## Unit Tests
- Rust notes::tasks regressions: rerun with explicit existing-task identities and rewritten titles after appending work; missing/unknown identities; new outcomes sharing an existing sentence; preserve started, done and archived task identities; duplicate excerpt/title pairs within a reply; separate outcomes sharing a sentence; distinct excerpts; existing-task prompt context.
- Note task status tests cover every SessionStatus, missing chat details, manual done and archived states, and latest linked chat selection.

## Integration / Functional Tests
- NotesScreen: Start persists its chat link; row replaces Start with Open chat; live session updates change the status; done and reopen persist; pending starts cannot duplicate.
- Browser mock extraction retains task identity after title edits and appended work, matching production's excerpt-based identity.

## Smoke Tests
- bun run build.
- bun run test, including frontend, sidecar, release and Rust workspace tests.

## E2E Tests
- Fresh-profile local browser: open a note, extract tasks, append a new task and extract again, verify unique rows; start one and verify Open chat and status; mark done and reopen.
- The browser uses the local mock backend; Rust tests cover actual model reply acceptance and durable writes.

## Manual / cURL Tests
- In the desktop app, add a bullet to a previously converted note and rerun extraction. Existing task ids, edits, states and chat links must survive.
- Start a task, return to Notes during and after its turn, and open the linked chat. Verify the row status changes and there is no Start action beside it.
- N/A for cURL: this change introduces no HTTP endpoint.
