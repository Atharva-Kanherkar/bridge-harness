# Security policy

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report them through [GitHub Security Advisories](https://github.com/Atharva-Kanherkar/bridge-harness/security/advisories/new) (Report a vulnerability on the repository **Security** tab). That keeps details private until a fix is ready.

Include:

- A clear description of the issue and impact
- Steps to reproduce, or a proof of concept if you have one
- Affected versions or commits, if known

We will acknowledge receipt, investigate, and work on a fix. We may ask for more detail. Credit in the advisory is given when you want it.

## What Bridge treats as sensitive

Bridge is a **local-first** desktop app. Design assumptions that matter for security reviews:

- **Provider credentials** — Sign-in runs each vendor's own login flow. Bridge does not collect, store, or log your provider API keys, OAuth tokens, or CLI credential file contents. Health and auth probes use status only, not secret values.
- **Your code** — Repositories and worktrees live on your machine. Bridge isolates task work in Git worktrees; policy gates risky commands and writes.
- **Local trust** — The runtime (`bridged`), SQLite data directory, and Unix socket handshake assume a trusted local user on the machine. Bridge is not hardened as a multi-tenant network service; do not expose its data directory or daemon socket to untrusted networks without additional controls.
- **Telemetry** — See [`docs/observability.md`](docs/observability.md) for what is recorded locally and what is deliberately excluded (credentials, raw provider secrets, etc.).

## Supported versions

Security fixes are applied on the current `main` branch and released builds as maintainers are able. Download current macOS builds from [GitHub Releases](https://github.com/Atharva-Kanherkar/bridge-harness/releases).

## Safe harbor

We appreciate responsible disclosure. We will not pursue legal action against researchers who report issues in good faith and avoid privacy violations, destruction of data, or disruption of user systems.
