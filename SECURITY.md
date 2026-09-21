# Security policy

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report them through **[GitHub Security Advisories](https://github.com/Atharva-Kanherkar/bridge-harness/security/advisories/new)** (Report a vulnerability on the repository Security tab). That keeps details private until a fix is ready.

Include:

- What you believe is affected (Bridge app, `bridged` daemon, browser extension, protocol surface, etc.)
- Steps to reproduce or a proof of concept
- Impact you expect (local data exposure, privilege escalation, remote code execution, etc.)
- Your environment (Bridge version, macOS version, which agents are enabled)

We will acknowledge receipt, investigate, and coordinate disclosure. We appreciate responsible reports.

## Product security posture (local trust)

Bridge is a **local-first** desktop control room for coding agents:

- **Your code and credentials stay on your machine.** Sign-in runs each vendor's own login flow; Bridge does not collect, store, or log provider credentials (see the [README](README.md)).
- **One owner per data directory.** The daemon and desktop app coordinate through a file lease so two processes cannot corrupt the same SQLite store.
- **Risky actions pause for approval.** Commands, writes, and delegations outside agreed scope require explicit user consent in the UI.
- **Isolated workspaces.** Task worktrees keep agent experiments off your main checkout.

This model assumes you trust the Bridge binary you installed, the Git repositories you attach, and the agent CLIs you enable. It does **not** replace your judgment about running untrusted projects or pasting secrets into chats.

## Supported versions

Security fixes are prioritized on the **latest release** on [GitHub Releases](https://github.com/Atharva-Kanherkar/bridge-harness/releases). Older builds may not receive backports; upgrade when we publish a fix.

## Out of scope for this repository

- Vulnerabilities in third-party agent CLIs or cloud APIs — report those to the respective vendors.
- Social engineering or phishing that does not exploit Bridge itself.
- Issues that require physical access to an unlocked machine with Bridge already signed in (treat as device compromise).

Thank you for helping keep Bridge users safe.
