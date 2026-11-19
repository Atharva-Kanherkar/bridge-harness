# Test contract: GitHub pane review, media and motion

Branch `feat/github-pane-review-media`. Locked before the PR opens.

## Resolve conversations

- `resolveThread` sends the thread's node id as a GraphQL variable and drops the cached threads (`resolving_a_conversation_sends_the_thread_id_as_a_graphql_variable`).
- An id that is not an opaque node id never reaches `gh` (`a_forged_conversation_id_never_reaches_gh`).
- The pane offers Resolve or Unresolve only when GitHub says the viewer may, flips the card at once, and puts it back with the reason when GitHub refuses (`GitHubPane.test.tsx`: "resolves and reopens a conversation…", "offers Resolve only when…").
- A resolved conversation folds to one line; the filter shows All, Open, Resolved.

## Comment

- A conversation comment, a thread reply and a diff line comment each post on their own button press with `confirmed: true`; no second sheet. A refusal stays on the composer and the draft is kept.
- A line comment carries `(path, line, side)`: a deleted row is the old file (`left`), anything else the new file (`right`); hunk headers offer nothing (`patch.test.ts`, "comments on a changed line…").
- The backend anchors a line comment to the head commit GitHub reports at that moment (`a_line_comment_is_anchored_to_the_current_head_commit`).
- Merge, close, reopen, approve, request changes, labels, ready, re-run and restore keep the confirmation sheet.
- The policy replay fixture has approve and deny rows for both new actions; a denied action spawns nothing.

## Images and videos

- `splitGithubMedia` lifts `![]()`, `<img>`, linked images, `<video>`, bare attachment URLs and video files into media rows; code spans, fences and non-http addresses are never lifted (`githubMarkdown.test.ts`).
- A screenshot table becomes captioned rows; a table with prose is untouched.
- Media on a GitHub host loads on its own with `referrerpolicy="no-referrer"`; any other host waits for a click.
- A failed direct load falls back once to `github/github_media`; a second failure shows a notice with Retry and a link out.
- `github/github_media` fetches only https on GitHub-owned hosts, holds every redirect hop to the same rule, sends the token only to the repository host and `raw.githubusercontent.com`, caps the size, and returns only bytes that sniff as an image or video (`media_is_fetched_only_from_github…`, `media_type_comes_from_the_bytes…`).

## Motion

- Thread cards and comments enter with a short stagger; a new comment rises alone; the thread body animates its height; the Resolved badge springs in; the filter pill glides.
- Reduced motion lands every one of these on its final frame.

## End to end

Run against real pull requests through `bridged --listen` and the web client, in Chrome with a fresh profile: read a PR with images, unresolve and resolve a real conversation, post and delete a reply containing a video, post and delete a line comment, and fetch an attachment through `github/github_media`.
