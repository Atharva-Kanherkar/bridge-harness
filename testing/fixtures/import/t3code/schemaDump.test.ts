// Rebuilds the T3 Code schema fixtures from T3 Code's own migrations.
// Copy into a t3code checkout at apps/server/src/persistence/ and run
//   T3_SCHEMA_OUT=<this folder> npx vitest run src/persistence/schemaDump.test.ts
// Structure only: no rows are written.
import { describe, it } from "@effect/vitest";
import * as NodeSqliteClient from "@t3tools/shared/nodeSqliteClient";
import * as Effect from "effect/Effect";
import * as NodeFS from "node:fs";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";
import { DatabaseSync } from "node:sqlite";

import { runMigrations } from "./Migrations.ts";

const HEADS = [5, 17, 23, 36, 43, 48, 54, 55, 63, 76, 87, 95, 102, 119];
const out = process.env.T3_SCHEMA_OUT ?? NodeOS.tmpdir();

describe("schema dump", () => {
  for (const head of HEADS) {
    it.effect(`head ${head}`, () => {
      const file = NodePath.join(NodeFS.mkdtempSync(NodePath.join(NodeOS.tmpdir(), "t3-schema-")), "state.sqlite");
      return Effect.gen(function* () {
        yield* runMigrations({ toMigrationInclusive: head });
      }).pipe(
        Effect.provide(NodeSqliteClient.layer({ filename: file })),
        Effect.tap(() =>
          Effect.sync(() => {
            const db = new DatabaseSync(file, { readOnly: true });
            const rows = db
              .prepare(
                "SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'view' THEN 2 ELSE 3 END, rowid",
              )
              .all() as Array<{ type: string; name: string; sql: string }>;
            const ids = db
              .prepare("SELECT migration_id, name FROM effect_sql_migrations ORDER BY migration_id")
              .all() as Array<{ migration_id: number; name: string }>;
            db.close();
            // FTS shadow tables are created by their virtual table; skip them.
            const shadow = new Set(
              rows.filter((row) => /CREATE VIRTUAL TABLE/i.test(row.sql)).flatMap((row) =>
                ["_config", "_data", "_docsize", "_idx", "_content"].map((suffix) => `${row.name}${suffix}`),
              ),
            );
            const sql = rows
              .filter((row) => !shadow.has(row.name))
              .map((row) => `${row.sql.trim()};`)
              .join("\n");
            NodeFS.writeFileSync(NodePath.join(out, `schema-${head}.sql`), `${sql}\n`);
            NodeFS.writeFileSync(
              NodePath.join(out, `migrations-${head}.txt`),
              ids.map((row) => `${row.migration_id}\t${row.name}`).join("\n") + "\n",
            );
          }),
        ),
      );
    });
  }
});
