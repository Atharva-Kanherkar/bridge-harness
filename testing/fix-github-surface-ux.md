# fix/github-surface-ux - Test Contract

Follow-ups to the github-surface-scale fixes, all presentation-layer: the
sidebar duplicated the pane's PR list and is unwanted; loading states were
bare spinner notices (some not even spinning); GitHub-authored markdown
(PR/issue bodies, comments, review threads) rendered as plain preformatted
text.

## Functional Behavior

- The sidebar no longer renders a PULL REQUESTS section; the GitHub dock pane
  is the single surface for pull requests. CI-finished toasts still deep-link
  into the pane.
- PR/issue descriptions, conversation comments, issue comments, and review
  thread comments render as markdown through the app's existing `Markdown`
  component (React nodes only; fenced HTML confined to a sandboxed iframe) at
  the pane's compact type scale. Remote HTML stays inert exactly as before.
- Loading states are skeletons that mirror the shape of the content they
  precede: list rows while the PR/issue/repository reads are in flight, and a
  title/meta/body skeleton while a PR or issue detail loads. The initial
  availability probe shows a single spinning notice.
- Comments show a relative timestamp when their `createdAt` parses; an
  unparseable timestamp is omitted rather than rendered raw.
- The PR detail header names the author alongside state and number.

## Unit Tests

- Sidebar: renders no "PULL REQUESTS" section and never calls the GitHub list
  API.
- Pane: a pending pull-request list renders a skeleton (`role="status"`),
  not a spinner notice; a pending detail renders the detail skeleton.
- Pane: a PR body with markdown (bold, fenced code) renders `<strong>` and a
  code block; raw `<script>` in a body still never becomes an element.
- Pane: comments render markdown bodies and a relative timestamp; a comment
  whose `createdAt` does not parse shows no timestamp.

## Integration / Functional Tests

- Existing GitHubPane, App wiring, and sidebar suites stay green with the
  panel removed and the new loading markup.

## Smoke Tests

- `bun run check`, `bun run test`, and `bun run build` complete successfully.

## E2E Tests

N/A - jsdom component tests cover the removed section, skeletons, and
markdown rendering; there is no browser E2E harness for the native pane.

## Manual / cURL Tests

- Open the GitHub pane on a real repository: list skeleton, then rows; open a
  PR: detail skeleton, then a formatted description with working headings,
  lists, code fences, and links.
- Confirm the sidebar shows no pull-request rows in any workspace.
