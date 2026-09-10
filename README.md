<h1 align="center">Bridge</h1>

<p align="center">
  <strong>The control room for coding agents.</strong><br/>
  Run Codex, Claude Code, and OpenCode side by side — safely, on your Mac, with your own subscriptions.
</p>

<p align="center">
  <a href="https://github.com/Atharva-Kanherkar/bridge-harness/releases"><strong>Download for macOS</strong></a>
</p>

<p align="center">
  <sub>macOS 12+ · Apple Silicon · Early-stage, under active development</sub>
</p>

---

## What is Bridge?

Bridge is a native macOS app for working with AI coding agents on real projects. Point it at a Git repository, start a session with the agent you want, and get your work done — without living in the terminal and without worrying about an agent trampling your checkout.

Use your existing agent subscriptions. Your code and your credentials stay on your machine.

## Why Bridge?

- **One place for every agent.** Start Codex, Claude Code, or OpenCode sessions from the same window and switch between them freely.
- **Experiment without fear.** Each task gets its own isolated workspace, so agents can try things without touching your main branch.
- **Nothing gets lost.** Conversations are saved automatically. Rewind, fork, or resume a session days later — right where you left off.
- **You're always in charge.** Agents ask before running anything risky. You approve or decline in one click.
- **Know what you're spending.** See usage, rate limits, and context health per provider before you hit a wall.
- **It remembers how you work.** Save preferences and project knowledge once; Bridge surfaces the right bits in future sessions.

## What you can do with it

### Work with any of your agents
Chat with Codex, Claude Code, or OpenCode in a clean native UI — messages, reasoning, plans, tool calls, diffs, and approvals rendered properly instead of crammed into a terminal.

### Keep tasks safely separated
Spin up a task workspace per piece of work. Agents work in their own branch and folder; your main checkout stays clean. Run a second opinion in parallel when it matters.

### Stay in control of risky actions
Commands, file writes, and delegation requests outside the agreed scope pause for your approval. Review the exact diff before anything lands.

### Never lose a thread
Every session is stored locally and stays inspectable. Go back to an earlier point, fork the conversation to try a different approach, or resume after a restart — without losing what the agent already figured out.

### See costs and limits up front
Per-provider usage, rate-limit status, and context pressure are visible in the app, so you can switch models or wrap up before quality degrades.

### Give agents your login — safely, temporarily
Let an agent use a web page you're already logged into (Chrome or Safari) through a tab you explicitly approve. No passwords or cookies are copied anywhere.

### Keep project knowledge
Store the preferences, conventions, and decisions agents should follow. Recall them in any session, on any workspace.

### Stay connected to GitHub
Browse issues and pull requests, open work from a task, and keep the conversation tied to the code under review.

## Works with your subscriptions

| Agent | What you need |
| --- | --- |
| Codex | Install the `codex` CLI and sign in |
| Claude Code | Install the `claude` CLI and sign in (needs Node.js 18+) |
| OpenCode | Install the `opencode` CLI and sign in |

Missing something? That agent simply shows as unavailable — Bridge still starts and everything else keeps working. Bridge never collects or stores your provider credentials.

> [!TIP]
> Bridge can also install and manage these agent runtimes for you, so you don't have to set them up by hand. Your own installs always take precedence.

## Get started in 60 seconds

1. **Download Bridge** from [GitHub Releases](https://github.com/Atharva-Kanherkar/bridge-harness/releases) — open the `.dmg`, drag **Bridge** into Applications, and launch it.
2. **Add a project** — pick a local Git repository.
3. **Start working** — create a workspace, pick your agent and model, and send your first message.

That's it. Approvals, history, and usage tracking are on from the start.

## Download

- **Release builds:** [GitHub Releases](https://github.com/Atharva-Kanherkar/bridge-harness/releases) — look for `Bridge_*.dmg`
- **Requirements:** macOS 12 or later (Apple Silicon), Git, and Node.js 18+ (only needed for Claude sessions)
- **Updates:** in-app auto-update isn't in this release yet — grab new builds from Releases. See the [CHANGELOG](CHANGELOG.md) for what's new.

> [!NOTE]
> Bridge is early-stage software. Expect rough edges and frequent improvements. macOS may ask for file access the first time Bridge touches `Desktop`, `Documents`, or `Downloads` — keeping repos in a folder like `~/Code` avoids repeated prompts.

## Learn more

- [CHANGELOG](CHANGELOG.md) — what shipped in each release
- [docs/session-forest.md](docs/session-forest.md) — how history, rewind, and resume behave
- [docs/delegation-policy.md](docs/delegation-policy.md) — how supervised multi-agent work stays bounded
- [docs/compaction-and-resume.md](docs/compaction-and-resume.md) — checkpoints and session restoration
- [docs/authenticated-browser-bridge.md](docs/authenticated-browser-bridge.md) — the approved-tab browser model
- [docs/work-brief.md](docs/work-brief.md) — the daily work briefing

<details>
<summary><strong>Building from source & contributing</strong></summary>

```sh
git clone https://github.com/Atharva-Kanherkar/bridge-harness.git
cd bridge-harness
bun install
bun run dev        # fast frontend iteration (mock data, no Rust shell)
bun run tauri dev  # full desktop app
```

Before opening a PR:

```sh
bun run build
bun run test
```

Keep changes focused, follow [AGENTS.md](AGENTS.md) (Tailwind CSS v4 only, colocated tests), and use Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`).

</details>

## License

No license file is currently included in the repository. Treat the project as all rights reserved unless the maintainers provide separate written permission.
