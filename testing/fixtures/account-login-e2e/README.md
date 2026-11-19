# account-login-e2e

Fake vendor CLIs for `src-tauri/bridged/tests/account_login.rs`. Each one
behaves like its vendor's sign-in for an extra account: it prints a browser
link, then records the login in the folder Bridge pointed it at
(`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_DATA_HOME/opencode`). The usage
probes then answer with limits that depend on which folder they were run in,
so the test proves each account's limits come from its own login.

No network, no real credentials.
