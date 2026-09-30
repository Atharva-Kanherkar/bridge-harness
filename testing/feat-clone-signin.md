# feat/clone-signin — test contract

Locked before implementation. Part 3 (backend) of the browser-clones epic
(#752), stacked on parts 1 (#750) and 2 (#751). Extension-free sign-in import
and the narrow agent tool, in `bridge-core`, macOS only. Out of scope on
purpose: the approval card and any React/UI, the sign-in-inside-the-clone flow,
the live-turn capability injection, Safari.

## Shape of the thing

- `browser_clone_signin.rs` — `import_cookies(browser, profile, registrable_domain)`
  reads only the approved domain's cookies from the user's Chrome or Brave
  cookie store (read-only SQLite), decrypts v10 values via the system Keychain
  Safe Storage item, and returns `Vec<CookieSpec>` for `load_session`. It never
  opens the password (Login Data) store; values never appear in an error.
- `clone_browser_tool.rs` — `CloneBrowserTool` mints a per-session, per-clone,
  per-runtime-pid capability (unix socket + token, mirroring
  `browser_bridge::capability_context`) exposing only inspect, screenshot,
  click, type, scroll, navigate (same-domain), focus, and result. Every result
  is scrubbed of the clone's registered secrets before it returns.

## 1. Cookie import — `browser_clone_signin::tests`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | Domain filter excludes other sites and suffix tricks | `login.example.com` matches `example.com`; `example.com.evil.test` and `other.com` do not |
| 1.2 | The approved domain is validated | good domains pass; empty, dotless, leading/trailing dot, path chars, and a leading-hyphen label are rejected |
| 1.3 | A real v10 cookie round-trips | a value sealed the way Chromium seals it on macOS (PBKDF2-SHA1 `saltysalt`/1003/16, AES-CBC, space IV, `v10` prefix) decrypts back exactly — proving the CommonCrypto key derivation and AES link and run |
| 1.4 | The host-hash prefix is stripped | a payload prefixed with `SHA-256(host)` (newer Chromium) decrypts to the value without it |
| 1.5 | A non-v10 blob is refused | a value without the `v10` prefix is a value-free `Decrypt` error |

Not unit-tested (needs the real Keychain + a real profile, which prompts and
reads real cookies): the end-to-end `import_cookies` against a live browser
store. The crypto, domain filter, and validation around it are covered above;
the Keychain fetch is the one uncovered line and is the same
`generic_password` call the attached-tab bridge already uses.

## 2. The agent tool — `clone_browser_tool::tests`

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | No cookie/storage/eval kind exists | `cookie`, `storage`, `eval`, `Runtime.evaluate` are refused by `command` |
| 2.2 | Page mutations are gated | every mutating kind (click, type, scroll, navigate, focus) is a *valid* command but sits in `MUTATING_KINDS`, and `execute` refuses it while `allow_mutation` is false — the default — so nothing acts on the page until an approval path exists |
| 2.3 | Navigate is same-domain only | a foreign host and a look-alike suffix are refused; a subdomain of the approved domain is allowed |
| 2.4 | Results are scrubbed | a registered secret (and its encodings) in a browser result is replaced before return |

The capability binding (token + `X-Bridge-Session` + peer-pid descendant of the
runtime) mirrors `browser_bridge::capability_context`, which is already covered
by its own tests; this part reuses that shape.

## 3. Repo gates

- `bun run check`, `cargo test -p bridge-core --lib`, and `bun run build` are
  green. The four `terminal_workspace`/`api` PTY failures pre-exist on
  `origin/main` on this machine and are untouched.
- No protocol method and no generated-artifact change. All new code is behind
  `#[cfg(target_os = "macos")]`; the crate still checks on non-macOS.

## Review findings folded in

- **Adopted from a Codex worker, reviewed here.** The importer and tool were
  drafted by a delegated worker (it could compile but not open a PR), then
  reviewed, reformatted, and hardened in adoption.
- **Page actions must wait for the user.** The draft dispatched click, type,
  and navigate immediately, which contradicts the epic's rule that agent
  actions wait for approval. Since the approval card is a later part, mutating
  kinds are now refused by default (`allow_mutation` off) and only read kinds
  (inspect, screenshot, result) work; the card flips the flag (2.2).
- **The crypto was unproven.** The worker only ran a domain-filter test, so the
  decrypt path (and that CommonCrypto even links) was untested. Added a seal /
  decrypt round-trip that needs no Keychain or real cookie (1.3–1.5).

Explicitly **not** changed: the clone process and lifecycle (part 1), the leak
guard and egress proxy (part 2) — their tests still pass unchanged; the
attached-tab bridge; the protocol.
