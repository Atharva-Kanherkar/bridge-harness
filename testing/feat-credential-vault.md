# Contract: API keys an agent can use but never see

Supersedes items 9, 9a, 10 and the "OpenAI only" scope of `issue-42-chat-secret-interception.md`. Interception itself (items 1 to 8) is unchanged.

## Scope

- A key reaches Bridge two ways. Pasted into a chat, it is caught by `secret_interception`, replaced with `[secret:sec_…]`, and kept in memory for that chat. Saved in Settings, its value goes to the macOS Keychain and its placeholder is its name, `[secret:OPENAI_API_KEY]`.
- Every key has a list of sites (hosts). A pasted key starts on its kind's API: OpenAI `api.openai.com`, Anthropic `api.anthropic.com`, GitHub `api.github.com` and `uploads.github.com`, Slack `slack.com`, Notion `api.notion.com`. Other kinds start with none. Private keys and lone AWS access key ids are redacted, not kept.
- The agent calls `http://<proxy>/credential-proxy/<session>/<host>/<path>`. Bridge forwards to `https://<host>/<path>` and replaces each placeholder in a request header with the real key, only when that key is allowed on that host.
- The proxy pass reaches harness processes as `$BRIDGE_PROXY_PASS` in their environment, never through a prompt.
- Both hosts serve the proxy: the embedded shell with `/health`, and every `bridged` daemon from `Daemon::start`. `127.0.0.1:4317` when free, otherwise an ephemeral port, and the instructions name the bound one.

## Functional expectations

1. A placeholder in a request header becomes the key, including inside Basic credentials (`curl -u "user:[secret:KEY]"`), which are decoded, filled and encoded again. In the path, query or body it is sent literally.
2. A key used on a host it is not allowed on is refused with 403, naming the key, the host and where to allow it. Nothing is sent.
3. A call with no placeholder in any header is refused, so the proxy is not an open relay.
4. A chat cannot use another chat's pasted key. Saved keys work in every chat.
5. A saved key's old chat placeholder (`sec_…` it was saved from) keeps resolving, to the saved key's sites.
6. Any occurrence of a used key in the response body or headers is replaced by its placeholder (for Basic credentials, the encoded token too), and the response carries `x-bridge-redacted` naming it.
7. No redirects are followed. Hop-by-hop headers, `host`, `accept-encoding` and the pass header are not forwarded. `set-cookie` is not returned.
8. GET, POST, PUT, PATCH and DELETE are forwarded; anything else is 405. Bodies over 10 MiB are 413.
9. The old route `/credential-proxy/<session>/sec_…/v1/…` still reaches OpenAI for a pasted OpenAI key.
10. The pass is checked before a body is read or a thread is started.
11. The pass never appears in a stored session entry, the live event stream or a transcript export, even when a command or its output contains it.
12. The instructions list saved keys by name and site, never by value, and say when the proxy is not running. A running chat is told again when the saved keys change.
13. Settings lists saved keys (name, kind, sites) and this launch's pasted keys (kind, chat, sites). It can add a key, edit a saved key's sites, save a pasted key for every chat, paste a saved key again, and forget either kind. No API result carries a value.
14. A saved key this build cannot read back is listed with `needsValue`, and pasting it again replaces the Keychain item so the current build owns it.
15. In a user bubble, `[secret:…]` renders as a chip, not raw text.

## Automated verification

- `credential_broker` unit tests cover 1 to 10 and 12 against a local upstream, without network.
- `transcript_export::the_key_proxy_pass_never_reaches_history_or_an_export` covers 11 and fails without the scrub in `store::session_event_in_transaction`.
- `api::…::api_keys_save_promote_update_and_forget_without_returning_a_value` covers 13 and 14 in core.
- `bridged` `the_daemon_serves_the_api_key_proxy_its_sessions_are_told_about` covers the daemon host.
- `KeysPage.test.tsx` and `Markdown.test.tsx` cover 13 to 15 in the UI.

## Manual verification

With a fresh `bridged --data-dir` and `bridge exec --harness claude`:

1. Paste a Slack-shaped token and ask for `POST https://slack.com/api/auth.test`. Slack answers `invalid_auth`; it says `not_authed` when no token arrives.
2. Ask for the same token on `httpbin.org`. Bridge refuses, and the agent relays where to allow the site.
3. Save `E2E_ECHO_KEY` for `httpbin.org`, restart the daemon, and ask a new chat to call `/headers` with `curl -i`. The echo shows the placeholder and `x-bridge-redacted`.
4. `grep` the data directory and `~/.claude/projects` for the key and the pass: no hits.

## Explicit non-goals

- Request body or query-string substitution.
- AWS SigV4 re-signing, streaming responses, a TLS-terminating proxy.
- Linux and Windows key storage. Pasted keys still work there for the launch.
