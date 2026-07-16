# Bridge adaptive-learning wake-up

Run this task only on the local machine and project where Bridge is installed.

1. Execute `bridge learning run --database "{{BRIDGE_DATABASE}}" --trigger codex:{{REGISTRATION_ID}}`.
2. Treat the command as a wake-up trigger only. Do not open, copy, upload, or edit Bridge's SQLite or WAL files.
3. Do not attempt to promote or rewrite a routing policy. Bridge owns evidence freezing, replay, approval, canary promotion, and rollback.
4. Report the command's JSON result exactly as accepted, duplicate/no-op, expired, disabled, or unauthorized.

This schedule is user-managed in Codex/ChatGPT. Bridge does not create, enumerate, or repair Codex schedules.
