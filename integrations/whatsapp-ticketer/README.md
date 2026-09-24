# WhatsApp ticketer

A separate, always-on service for turning explicit Bridge feedback in one
WhatsApp group into a validated GitHub issue. It does not run inside `bridged`,
the desktop app, or a Bridge connector.

Send `/ticket the sidebar flickers when I switch chats`, start a message with
`@bridge`, or react 🐛 to a recently observed text message. The bot reacts ⏳,
drafts the issue, checks the repository's two-audience format, files it, then
replaces the reaction with 🎫 and quotes the source with the issue link.
Failures get ❌. A repeated trigger resends the existing link.

## Prerequisites

- Node **20.19+**, preferably Node 22 LTS, Bun, Git, `tar`, and authenticated `gh`.
  The pinned Baileys version requires Node 20; Node 18 is not supported.
- A dedicated WhatsApp account/number that is a member of the chosen group.
- One process on one host with a persistent local disk. Do not share its database
  or WhatsApp auth directory between replicas or across hosts.
- A local clone with an up-to-date `main` branch for repository evidence.
- Claude API access or Claude subscription login on the service account.

Baileys is an **unofficial linked-device client**. It may violate WhatsApp's
terms and the account may be restricted or banned. Use a dedicated account;
understand this risk before linking it. This is not the WhatsApp Cloud API.

## Setup

From the Bridge repository root:

```sh
bun install --frozen-lockfile
gh auth status
gh label create from-whatsapp --repo Atharva-Kanherkar/bridge-harness \
  --color 25D366 --description 'Feedback filed from the WhatsApp group'

# Example private disk layout, outside the repository checkout.
mkdir -p "$HOME/.local/share/bridge-ticketer/auth"
chmod 700 "$HOME/.local/share/bridge-ticketer/auth"
```

Create the label once with a maintainer account (skip if it already exists).
The running service never creates labels and drops unknown model-suggested labels.

Set these in the process environment or your host's secret manager. Do not
commit a credentials file. The service does not automatically read `.env` files.

| Variable | Value |
| --- | --- |
| `WA_AUTH_DIR` | Absolute private directory outside both checkouts, on persistent disk |
| `WA_GROUP_JID` | The one group JID, e.g. `120000000000000000@g.us` |
| `WA_ALLOWED_SENDERS` | Comma-separated individual JIDs, e.g. `919000000001@s.whatsapp.net,123456789@lid` |
| `TICKETER_DB` | Absolute SQLite file path on persistent disk, outside both checkouts |
| `REPO_CLONE_DIR` | Local Bridge clone with a `main` ref; only committed `main` content is exposed |
| `GH_TOKEN` | Fine-grained token for this repository only, Issues: read/write (or existing `gh` login) |
| `ANTHROPIC_API_KEY` | Claude API key, unless using subscription authentication |
| `CLAUDE_CODE_OAUTH_TOKEN` | Optional subscription token supported by the Claude SDK; local Claude login also works |
| `TICKETER_MODEL` | Optional model override; defaults to `claude-sonnet-5` |

Use group/participant JIDs from your existing WhatsApp linked-device admin
tooling. Baileys can identify a participant by a phone JID or a privacy LID;
configure their exact trusted identity. The listener also recognizes Baileys'
participant phone/LID alternatives. It does not print participant identifiers
or provide a discovery mode that listens to unrelated groups.

```sh
npm start --prefix integrations/whatsapp-ticketer
```

First launch prints a QR code. On the dedicated WhatsApp account, open **Linked
devices → Link a device** and scan it. Keep this terminal/log private. Subsequent
launches reuse `WA_AUTH_DIR`. Transient disconnects reconnect with backoff;
logout, a bad session, or a replaced connection requires operator intervention.

For a spare Mac, supervise this command with `launchd`. For Railway, deploy
from the repo root, install the prerequisites above, use this start command,
and mount a persistent volume for both auth and database paths. Set one replica.
The service uses outbound connections and does not need an HTTP listener.
Update the evidence clone's `main` and restart to refresh the agent's snapshot.
Do not run a checkout/pull while a snapshot is being prepared.

## Boundaries and operation

- Only allowlisted senders in the configured group are cached or can trigger a
  model call. Messages sent by the bot itself are ignored. Normal chat never
  files issues. History/append events may supply context but cannot file issues.
- Text only. Media captions, images and voice notes do not trigger filing.
  Prefix triggers must start at the first character. `@bridge` is literal text,
  not WhatsApp's numeric mention syntax.
- Context is the source, an allowlisted same-group quote, and up to ten earlier
  messages from the preceding 15 minutes. Numeric phone strings and JIDs are
  redacted before inference and again in generated titles, bodies, and labels.
  Unknown/numeric display names become “Group member”. Redaction deliberately
  removes long numeric strings even if they might be non-phone identifiers.
- Recent sanitized messages and their transport keys are retained privately for
  24 hours, capped at 1,000 messages. 🐛 works on messages this service observed
  during that period, including across restarts. It cannot fetch arbitrary old
  WhatsApp history. Repost old feedback with `/ticket` if it is no longer cached.
  Quoted messages still provide context when they were not cached.
- The agent reads a temporary archive of tracked `main` files, with symlinks
  removed. Only Read/Grep/Glob are exposed and every tool call is path-checked
  by a PreToolUse hook. Shell, writes, MCP, plugins and inherited settings are
  unavailable; GitHub/WhatsApp credentials are removed from its environment.
  Chat and repository contents are treated as untrusted evidence.
- Each draft uses at most 12 model turns and a $2 SDK budget. Invalid issue
  format gets one repair attempt under the same 80-second processing deadline.
  The service serializes jobs with a bounded queue of 20; the under-90-second
  receipt target applies to an idle healthy service, not backlog, outages or
  provider delays. Overflow triggers are ignored; resend when the bot is idle.
- Only the service invokes `gh issue create`, using a private temporary body
  file and argument arrays. The target repository is fixed. Exemption markers
  and the `format-exempt` label are rejected. No chat action closes, edits, or
  comments on issues.
- Filed receipts survive restart; failed WhatsApp delivery cannot recreate the
  issue. Source identifiers are hashed before adding a reconciliation marker
  to GitHub. The persistent database contains private transport keys: protect it
  and backups like the auth directory.

## Recovery

If a GitHub create times out or the process crashes after submission, its state
remains `creating`. Re-triggering searches all issue states for the exact hashed
receipt and returns the existing link. GitHub search can lag. If it has no match,
the bot refuses to create again and asks for operator review. After confirming
that GitHub did **not** create an issue, stop the service, back up the database,
inspect `SELECT id, state FROM tickets WHERE state = 'creating';` with SQLite,
and change only that confirmed row's state to `failed`. Restart and re-trigger.
Never clear a `creating` row merely because search has not indexed it yet.

A process lock prevents two local instances using the same database. Dead local
PID locks are recovered automatically. After moving a volume to a new host,
inspect/remove a stale `.lock` only after confirming no other instance uses it.
On startup failure, check required variables, disk permissions, `main`,
`gh auth status`, and the `from-whatsapp` label. Logs omit raw exceptions because
provider/transport errors can contain private message text or credentials.

## Verification

```sh
node --test integrations/whatsapp-ticketer/test/*.mjs
bun run build
bun run test
```

Tests use fake model, GitHub and WhatsApp boundaries; they never link a device,
call a paid model, or file an issue. They exercise actual SQLite persistence,
archive isolation, real Baileys event shapes, filtering, privacy, repair,
concurrent triggers, crash reconciliation and receipt retries. CI runs these
tests in the sidecar job. After deployment, verify one approved test message in
the configured group and confirm the issue stays open after the format workflow.
