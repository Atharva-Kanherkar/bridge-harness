# fix/github-inline-tab — test contract

Locked before implementation. Implements #566: a clicked GitHub link that the
inline GitHub pane can render opens in the pane, not in the OS browser.

## Shape of the thing

Bridge has had both halves of this for a while and they have never met. The
dock's `github` pane renders pull requests, issues and a repository overview
from the workspace's own repo, and `App` can already address it —
`openPullRequestPane` bumps a nonce, sets an intent, and dispatches
`open-pane: github`. Meanwhile `installExternalLinkHandler` captures every
`a[href]` click in the document and hands the URL to the OS default browser
with no host inspection at all. So the PR an agent linked in the transcript
leaves the app, and the pane one tab away — which renders that exact PR,
with its diff, its checks and its review threads — stays empty.

The fix gives the interceptor a router to ask. `externalLinks` gains a
settable internal router; `openExternalUrl` consults it before falling back
to the shell, so the two direct callers (Work-task evidence, the Codex setup
button) route by the same rule as every anchor. `App` registers a router that
parses the URL, checks it against the workspace's own repository, and turns a
match into a pane intent.

Three things keep it honest. The pane resolves its repository server-side
from the workspace's Git remote and never takes a repo selector from a caller
(`feat-343-github-surface-actions`), so a link to *any other* repository
cannot be shown inline and keeps going to the browser. Resolving the repo
costs a `gh` round-trip, so it happens on the first GitHub-shaped click and
is remembered per workspace — a chat whose links never point at GitHub never
pays for it. And the affordances whose entire purpose is to leave the app —
the pane's own "Open on GitHub", the truncated-patch "view the full diff",
the per-check "Open logs" — say so on the anchor and are never routed.

The intent widens from a bare PR number to a view, because the pane has more
than one view and a URL names which. `openPullRequestPane` keeps its
signature and its caller (the CI toast); it just builds the wider intent.

## 1. Parsing a GitHub URL — `src/githubLinks.test.ts` (new)

`parseGithubLink` maps a URL to the pane view that renders it, or null.

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | A pull request | `/o/r/pull/12` → `{ host: "github.com", owner: "o", name: "r", view: { kind: "pull", number: 12, tab: "conversation" } }` |
| 1.2 | Its files and checks name a sub-tab | `/pull/12/files` → tab `changes`; `/pull/12/checks` → tab `checks` |
| 1.3 | Sub-paths without their own view still land on the PR | `/pull/12/commits` and `/pull/12/commits/abc123` → tab `conversation` |
| 1.4 | Query and fragment are not part of the route | `/pull/12/files?w=1#diff-a` → pull 12, tab `changes` |
| 1.5 | An issue | `/o/r/issues/204` → `{ kind: "issue", number: 204 }` |
| 1.6 | The three list/overview shapes | `/o/r/pulls` → `pulls`; `/o/r/issues` → `issues`; `/o/r` and `/o/r/` → `repository` |
| 1.7 | Views the pane does not have stay null | `/o/r/commit/abc`, `/o/r/blob/main/a.ts`, `/o/r/tree/main`, `/o/r/releases`, `/o/r/actions/runs/1`, `/o/r/discussions/3`, `/o/r/wiki`, `/o`, `/` |
| 1.8 | A non-numeric or zero number is not a number | `/pull/abc`, `/pull/`, `/issues/0`, `/pull/1.5`, `/pull/-1` → null |
| 1.9 | Only http(s) parses | `javascript:`, `file:///o/r/pull/1`, `mailto:`, `not a url` → null |
| 1.10 | The host is carried, lowercased, and a GHE host parses like any other | `https://GITHUB.COM/o/r/pull/1` → host `github.com`; `https://ghe.corp/o/r/pull/1` → host `ghe.corp` |
| 1.11 | A trailing `.git` on the repo is not part of its name | `/o/r.git/pull/1` → name `r` |

`githubLinkMatchesRepository` decides whether the pane could show it.

| # | Behaviour | Assertion |
|---|---|---|
| 1.12 | Host, owner and name must all agree | the matching repository matches; a different owner, a different name, and a different host each do not |
| 1.13 | GitHub is case-insensitive about owner and repo | `Owner/Repo` matches `owner/repo` |
| 1.14 | No repository resolved means no match | `null` and `undefined` do not match |

## 2. The router seam — `src/externalLinks.test.ts` (extended)

Every existing case stays green unchanged.

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | A router that takes the URL keeps it in the app | with a router returning true, `openExternalUrl` does not call the shell |
| 2.2 | A router that declines falls through | returning false, the shell is called with the URL |
| 2.3 | The router may answer asynchronously | a router resolving true after a tick still suppresses the shell |
| 2.4 | A throwing router never strands the link | the shell is still called |
| 2.5 | An unroutable scheme never reaches the router | `javascript:alert(1)` calls neither the router nor the shell |
| 2.6 | Clearing it restores the old behaviour | after `setInternalLinkRouter(undefined)` the shell is called again |
| 2.7 | The interceptor routes anchor clicks | a clicked `a[href]` reaches the router and is not handed to the shell |
| 2.8 | A deliberate escape is never routed | an anchor carrying `data-system-browser` goes to the shell with the router untouched |
| 2.9 | `openInSystemBrowser` bypasses the router | with a router returning true, the shell is still called |
| 2.10 | The interceptor still declines what it always declined | right/middle click, an already-prevented event, and a non-external href leave both untouched |

## 3. Routing into the pane — `src/App.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | A PR link for this repo opens the pane on that PR | clicking an anchor to the workspace repo's `/pull/N` opens the dock on the GitHub pane and the shell is not called |
| 3.2 | A link to another repository still leaves | an anchor to a different `owner/name` calls the shell and does not open the pane |
| 3.3 | A GitHub path with no inline view still leaves | a `/commit/<sha>` anchor calls the shell |
| 3.4 | A chat with no worktree still leaves | with no repository workspace, the same PR anchor calls the shell |
| 3.5 | The repository is resolved once per workspace | two GitHub link clicks in one workspace issue one `github/github_status` read |

## 4. The pane obeys the wider intent — `src/components/GitHubPane.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | A pull intent opens that PR's detail | the detail for the intent's number renders |
| 4.2 | A pull intent naming a sub-tab selects it | `tab: "changes"` renders the Changes panel selected; `tab: "checks"` the Checks panel |
| 4.3 | An issue intent opens that issue | the Issues tab is selected and the issue detail renders |
| 4.4 | A list intent selects the tab and clears any selection | `pulls`, `issues` and `repository` intents each render their surface with no detail open |
| 4.5 | A repeated intent re-fires on a new nonce, not on a re-render | re-rendering with the same nonce does not refetch; a new nonce does |

## 5. The escapes stay escapes — `src/components/GitHubPane.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 5.1 | "Open on GitHub" is marked as leaving | the PR header anchor carries `data-system-browser` |
| 5.2 | The truncated-patch full-diff link is marked | the "view the full diff on GitHub" anchor carries it |
| 5.3 | A check's log link is marked | the per-check log anchor carries it — a log URL shaped like `/pull/N/checks` must not be swallowed by the pane |

Explicitly **not** changed: the pane's own views, data, polling, mutations,
confirmations and review threads; the `gh`-backed Rust surface and every
`github/*` RPC shape; the CI toast's payload and its cross-workspace hop; the
Tauri `shell:allow-open` capability scope; `Markdown`'s anchor rendering; and
`repoCloneTarget`, which still reads a bare `owner/repo` URL typed into the
composer as a clone target. Cross-workspace routing — a link whose repository
some *other* open workspace has checked out — remains out of scope and goes
to the browser. The design-system guard stays green with no new allowlist
entries.
