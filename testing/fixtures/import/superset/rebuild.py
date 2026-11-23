#!/usr/bin/env python3
"""Rebuild Superset's host.db and chat.db schemas at chosen migration heads,
with Superset's own drizzle migrations, for the import fixtures.

    python3 -I testing/fixtures/import/superset/rebuild.py <superset checkout>

Writes host-<idx>.sql / chat-<idx>.sql (schema) and host-<idx>.txt /
chat-<idx>.txt (the `__drizzle_migrations.created_at` values a database at
that head holds, one per line) beside this script. Structure only, no rows.
"""
import json, os, sqlite3, sys

source = os.path.abspath(sys.argv[1])
here = os.path.dirname(os.path.abspath(__file__))
SETS = {
    "host": ("packages/host-service/drizzle", [0, 6, 17, 20, 26, 33, 36, 38]),
    "chat": ("packages/chat-runtime/src/db/drizzle", [0, 1, 3, 4, 5]),
}
for name, (folder, heads) in SETS.items():
    folder = os.path.join(source, folder)
    entries = json.load(open(os.path.join(folder, "meta/_journal.json")))["entries"]
    for head in heads:
        db = sqlite3.connect(":memory:")
        db.execute("CREATE TABLE __drizzle_migrations(id INTEGER PRIMARY KEY, hash text NOT NULL, created_at numeric)")
        applied = []
        for entry in entries[: head + 1]:
            sql = open(os.path.join(folder, entry["tag"] + ".sql")).read()
            for statement in sql.split("--> statement-breakpoint"):
                if statement.strip():
                    db.execute(statement)
            db.execute("INSERT INTO __drizzle_migrations(hash, created_at) VALUES('h', ?)", (entry["when"],))
            applied.append(str(entry["when"]))
        schema = [row[0] + ";" for row in db.execute("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type DESC, name")]
        with open(os.path.join(here, f"{name}-{head}.sql"), "w") as out:
            out.write("\n".join(schema) + "\n")
        with open(os.path.join(here, f"{name}-{head}.txt"), "w") as out:
            out.write("\n".join(applied) + "\n")
        print(name, head, entries[head]["tag"], len(schema), "objects")
