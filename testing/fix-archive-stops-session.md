# fix/archive-stops-session Test Contract

## Functional Behavior
- Archiving a live, ready, starting, waiting, or stopped chat automatically ends its runtime before reclaiming its checkout. No separate Stop action is required.
- Descendant sessions hidden by a root archive are stopped too; unrelated siblings keep running.
- Preserve conversation history and dirty checkouts; reclaim only eligible owned checkouts.
- The UI keeps the existing archive confirmation, explains automatic shutdown, removes the archived family on success, and shows “Chat archived”. Retained worktrees are a warning within that confirmation.
- Failed archive calls leave the chat visible and show the error. Repeated clicks while archiving issue one request.
- Lifecycle conflicts fail without hiding the chat. Archived sessions cannot restart until restored.
- Delayed worker results retain their canonical history but never enqueue model input for an archived family. Restoring before or after the outbox sweep cannot replay cancellation notices from archive.

## Unit Tests
- Rust archive tests cover live status variants, stale adapter claims, real adapter shutdown, queued input cancellation, descendants, lifecycle conflicts, and archived restart refusal.
- Existing archive/history/worktree preservation tests remain green.
- Flush pending worker results after archive and after immediate restore; verify no family input is queued, result evidence remains, and unrelated delivery still works. Cover a parent hidden by an archived ancestor.

## Integration / Functional Tests
- Mount App and archive from the sidebar. Verify confirmation copy, one archive request, success notification, immediate removal without waiting for refresh, and failure preservation.
- Verify archiving another chat preserves the currently selected conversation.

## Smoke Tests
- `bun run build`
- `bun run test`

## E2E Tests
- N/A: desktop runtime shutdown is covered by Rust integration tests and UI wiring by App tests.

## Manual / cURL Tests
- In the desktop app, start a chat, archive it from its row menu, and accept the confirmation. It disappears and shows Chat archived without a Stop-first error.
- Archive a chat with unsaved files: history is archived, files stay, and the confirmation explains why.
- Restore from Settings > Archived chats: history returns without automatically restarting the provider.
- Desktop smoke steps require the packaged app; report separately if not executed.
