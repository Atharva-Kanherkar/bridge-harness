"use client";

import { Check, FileText, LoaderCircle, MousePointer2, Pencil, SquareTerminal, X } from "lucide-react";
import HarnessMark from "../app/HarnessMark";
import { typed, tween, usePlayback } from "./useInView";

/*
 * The feature visuals. Each one is a short loop of the app at work: replies stream, tools
 * spin and settle, diffs count up, statuses move on. Every panel is a pure function of one
 * tick counter from `usePlayback`, so the server and the client draw the same frame and the
 * loop restarts cleanly. Markup and type scale are lifted from the app.
 */
const frame = "relative overflow-hidden rounded-xl border border-border-card bg-background shadow-[0_0_0_1px_#000,0_30px_90px_-30px_rgba(0,0,0,0.9)]";
const appear = "animate-entry-in motion-reduce:animate-none";

function Caret() {
  return <span className="ml-px inline-block h-[1.05em] w-px translate-y-[0.18em] bg-foreground animate-caret motion-reduce:animate-none" />;
}

function Spinner({ size = 12 }: { size?: number }) {
  return <LoaderCircle size={size} className="shrink-0 animate-spin text-muted-foreground motion-reduce:animate-none" aria-hidden="true" />;
}

/** One activity row the way the transcript draws it: icon, label, and a spinner until it settles. */
function Tool({ kind, label, running, meta }: { kind: "read" | "edit" | "run"; label: string; running?: boolean; meta?: React.ReactNode }) {
  const Icon = kind === "read" ? FileText : kind === "edit" ? Pencil : SquareTerminal;
  return (
    <div className={`flex items-center gap-1.5 text-[11.5px] text-muted-foreground ${appear}`}>
      <Icon size={11} className="shrink-0" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate font-mono">{label}</span>
      {meta}
      {running ? <Spinner size={11} /> : <Check size={11} className="shrink-0 text-faint" aria-hidden="true" />}
    </div>
  );
}

function Diff({ add, del }: { add: number; del: number }) {
  return (
    <span className="shrink-0 font-mono text-[10.5px] tabular-nums">
      <span className="text-success">+{Math.round(add)}</span> <span className="text-destructive">−{Math.round(del)}</span>
    </span>
  );
}

/* Mission Control ------------------------------------------------------------------------ */

const fleet = [
  { harness: "claude", title: "Rotate refresh tokens", branch: "feat/token-rotation", file: "auth/session.ts", test: "bun test auth", add: 142, del: 38, end: "done", phase: 0 },
  { harness: "codex", title: "Fix flaky checkout test", branch: "fix/checkout-flake", file: "checkout.spec.ts", test: "bun test checkout", add: 12, del: 4, end: "needs you", phase: 34 },
  { harness: "opencode", title: "Paginate audit log", branch: "feat/audit-pages", file: "audit/list.rs", test: "cargo test audit", add: 88, del: 21, end: "done", phase: 70 },
  { harness: "cursor", title: "Settings copy pass", branch: "chore/settings-copy", file: "Settings.tsx", test: "bun run lint", add: 30, del: 30, end: "done", phase: 18 },
  { harness: "claude", title: "Postgres 17 migration", branch: "chore/pg17", file: "migrations/0042.sql", test: "sqlx migrate run", add: 64, del: 2, end: "done", phase: 52 },
  { harness: "codex", title: "Cache avatar lookups", branch: "perf/avatar-cache", file: "avatars/cache.go", test: "go test ./avatars", add: 47, del: 11, end: "needs you", phase: 96 },
];
const FLEET_LOOP = 150;

function tileState(t: number, phase: number) {
  const local = (t + phase) % FLEET_LOOP;
  return { local, step: local < 8 ? 0 : local < 22 ? 1 : local < 40 ? 2 : local < 62 ? 3 : local < 90 ? 4 : 5 };
}

/** A fleet of chats, each on its own branch, each at a different point in its own loop. */
export function Parallel() {
  const { ref, t } = usePlayback<HTMLDivElement>(FLEET_LOOP);
  const states = fleet.map(tile => tileState(t, tile.phase));
  const waiting = fleet.filter((tile, i) => tile.end === "needs you" && states[i].step === 5).length;

  return (
    <div ref={ref} className={`${frame} p-3`} aria-hidden="true">
      <div className="flex items-center gap-2 px-1 pb-3">
        <span className="text-[13px] font-semibold text-foreground">Mission Control</span>
        <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10.5px] text-foreground">{fleet.length} live</span>
        {waiting > 0 && <span className={`rounded bg-warning/15 px-1.5 py-0.5 font-mono text-[10.5px] text-warning ${appear}`}>{waiting} need you</span>}
        <span className="ml-auto text-[11px] text-faint">one worktree each</span>
      </div>

      <div className="grid grid-cols-2 gap-2 lg:grid-cols-3">
        {fleet.map((tile, i) => {
          const { local, step } = states[i];
          const status = step === 5 ? tile.end : step === 4 ? "verifying" : step === 0 ? "planning" : "working";
          const tone = status === "needs you" ? "text-warning" : status === "done" ? "text-muted-foreground" : status === "verifying" ? "text-info" : "text-success";
          const add = step >= 2 ? tween(local, 22, 40, 0, tile.add) : 0;
          const del = step >= 2 ? tween(local, 22, 40, 0, tile.del) : 0;

          return (
            <div key={tile.branch} className={`flex h-[176px] min-w-0 flex-col rounded-md border bg-card p-2.5 ${status === "needs you" ? "border-warning/50" : "border-border"}`}>
              <div className="flex items-center gap-1.5">
                <span className={`size-1.5 shrink-0 rounded-full ${status === "needs you" ? "bg-warning" : status === "done" ? "bg-faint" : "bg-success motion-safe:animate-pulse"}`} />
                <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-foreground">{tile.title}</span>
                <HarnessMark harness={tile.harness} size={11} />
              </div>
              <span className="mt-1 truncate font-mono text-[10px] text-faint">{tile.branch}</span>

              <div className="mt-2 flex min-h-0 flex-1 flex-col justify-end gap-1.5 overflow-hidden">
                {step === 0 && (
                  <p className="text-[11.5px] leading-5 text-body">
                    {typed(`Looking at ${tile.file} first.`, local, 0, 4)}
                    <Caret />
                  </p>
                )}
                {step >= 1 && step < 4 && <Tool key="read" kind="read" label={`Read ${tile.file}`} running={step === 1} />}
                {step >= 2 && <Tool key="edit" kind="edit" label={`Edited ${tile.file}`} running={step === 2} />}
                {step >= 3 && <Tool key="run" kind="run" label={tile.test} running={step === 3} />}
                {step >= 4 && (
                  <div key="review" className={`flex items-center gap-1.5 text-[11.5px] text-muted-foreground ${appear}`}>
                    {step === 4 ? <Spinner size={11} /> : <Check size={11} className="text-success" aria-hidden="true" />}
                    <span className="truncate">{step === 4 ? "Another model reviewing" : "Reviewed by another model"}</span>
                  </div>
                )}
              </div>

              <div className="mt-2 flex items-center justify-between border-t border-border pt-1.5">
                <span className={`text-[10px] uppercase tracking-[0.06em] ${tone}`}>{status}</span>
                {step >= 2 ? <Diff add={add} del={del} /> : <span className="font-mono text-[10.5px] text-faint">no changes</span>}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/* Switch harness ------------------------------------------------------------------------- */

/** Codex answers, you ask for a stronger model, the picker moves to Claude, and Claude carries on. */
export function SwitchHarness() {
  const { ref, t } = usePlayback<HTMLDivElement>(150);
  const codexReply = "Rotating on read means two concurrent reads can both mint a token. Rotation belongs in the write path.";
  const ask = "Keep going, but switch to a stronger model.";
  const claudeReply = "Picking up from the rotation plan. Moving the mint into the write path and adding a test for two concurrent readers.";

  const sent = t >= 52;
  const pickerOpen = t >= 56 && t < 70;
  const hover = t < 59 ? "GPT Terra" : t < 62 ? "Claude Sonnet" : "Claude Opus";
  const switched = t >= 70;
  const codex = ["GPT Luna", "GPT Terra", "GPT Sol"];
  const claude = ["Claude Sonnet", "Claude Opus", "Claude Haiku"];

  return (
    <div ref={ref} className={`${frame} flex h-[380px] flex-col justify-end p-4 sm:h-[400px]`} aria-hidden="true">
      <div className="flex flex-col gap-3 overflow-hidden px-1 pb-4">
        <div>
          <div className="mb-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <HarnessMark harness="codex" size={11} /> Codex · GPT Terra
          </div>
          <p className="text-[13px] leading-6 text-body">
            {typed(codexReply, t, 0, 4)}
            {t < 26 && <Caret />}
          </p>
        </div>

        {sent && <div className={`ml-auto w-fit rounded-2xl border border-border bg-accent/70 px-3.5 py-2 text-[13px] text-foreground ${appear}`}>{ask}</div>}

        {switched && (
          <div className={`flex items-center gap-2 text-[11px] text-faint ${appear}`}>
            <span className="h-px flex-1 bg-border" />
            Switched to Claude Code · Claude Opus. History kept.
            <span className="h-px flex-1 bg-border" />
          </div>
        )}

        {t >= 76 && (
          <div className={appear}>
            <div className="mb-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
              <HarnessMark harness="claude" size={11} /> Claude Code · Claude Opus
            </div>
            <p className="text-[13px] leading-6 text-body">
              {typed(claudeReply, t, 76, 4)}
              {t < 106 && <Caret />}
            </p>
            {t >= 108 && (
              <div className="mt-2">
                <Tool kind="edit" label="Edited auth/session.ts" running={t < 128} meta={<Diff add={tween(t, 108, 128, 0, 24)} del={tween(t, 108, 128, 0, 6)} />} />
              </div>
            )}
          </div>
        )}
      </div>

      <div className="relative">
        {pickerOpen && (
          <div className={`absolute bottom-12 right-0 z-10 w-[260px] overflow-hidden rounded-xl border border-border bg-popover shadow-2xl shadow-black/60 ${appear}`}>
            <div className="border-b border-border px-3 py-2 text-[12px] text-muted-foreground">Search models</div>
            <div className="px-2 py-2">
              {(
                [
                  ["codex", "Codex", codex],
                  ["claude", "Claude Code", claude],
                ] as const
              ).map(([id, label, models]) => (
                <div key={id}>
                  <div className="flex items-center gap-1.5 px-1.5 pb-1 pt-1">
                    <HarnessMark harness={id} size={11} />
                    <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">{label}</span>
                  </div>
                  {models.map(model => (
                    <div
                      key={model}
                      className={`flex h-7 items-center justify-between rounded-md px-1.5 text-[12.5px] transition-colors duration-150 ${
                        model === hover ? "bg-accent text-foreground" : "text-muted-foreground"
                      }`}
                    >
                      {model}
                      {model === (t >= 66 ? "Claude Opus" : "GPT Terra") && <Check size={12} aria-hidden="true" />}
                    </div>
                  ))}
                </div>
              ))}
            </div>
          </div>
        )}

        <div className="flex items-center gap-2 rounded-xl border border-border-card bg-card px-3 py-2.5">
          <span className="min-w-0 flex-1 truncate text-[13px] text-muted-foreground">
            {t >= 28 && t < 52 ? (
              <span className="text-foreground">
                {typed(ask, t, 28, 2)}
                <Caret />
              </span>
            ) : (
              "Send a follow-up…"
            )}
          </span>
          <span
            className={`inline-flex shrink-0 items-center gap-1.5 rounded-md border px-2 py-1 text-[12px] text-foreground transition-colors ${
              pickerOpen ? "border-ring bg-accent" : "border-border"
            }`}
          >
            <HarnessMark harness={switched ? "claude" : "codex"} size={12} />
            {switched ? "Claude Code · Claude Opus" : "Codex · GPT Terra"}
          </span>
        </div>
      </div>
    </div>
  );
}

/* Browser -------------------------------------------------------------------------------- */

const checks = [
  { name: "build", at: 66 },
  { name: "test", at: 76 },
  { name: "Vercel preview", at: 88 },
];

/** The agent asks for a signed-in tab, you allow it, and its cursor works the page live. */
export function Browser() {
  const { ref, t } = usePlayback<HTMLDivElement>(160);
  const url = "github.com/acme/web/pull/214";
  const approved = t >= 22;
  const loaded = t >= 42;
  const onChecks = t >= 58;

  // The agent's cursor walks from the middle of the page to the Checks tab, then to the preview link.
  const cursor = t < 46 ? { x: 55, y: 70 } : t < 100 ? { x: 47, y: 34 } : { x: 80, y: 86 };
  const clicking = (t >= 56 && t < 59) || (t >= 112 && t < 115);
  const narration =
    t < 44 ? "Opening the PR in a clone of your session" : t < 60 ? "Clicking Checks" : t < 96 ? "Waiting on 3 checks" : t < 114 ? "Preview is green. Opening it." : "Signed in on the preview, checking /settings";

  return (
    <div ref={ref} className={`${frame} flex h-[440px] flex-col p-4 sm:h-[480px]`} aria-hidden="true">
      <div className={`rounded-lg border border-border-card bg-card p-3 transition-opacity ${approved ? "opacity-70" : ""}`}>
        <div className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
          <HarnessMark harness="claude" size={12} />
          {approved ? "Browser approved for github.com" : "Claude Code wants a signed-in browser"}
          {approved && <Check size={12} className="ml-auto text-success" aria-hidden="true" />}
        </div>
        {!approved && (
          <div className="mt-2.5 flex gap-2">
            <span
              className={`inline-flex h-7 items-center rounded-md bg-primary px-2.5 text-[12px] text-primary-foreground transition-transform duration-150 ${t >= 18 ? "scale-95" : ""}`}
            >
              Allow once
            </span>
            <span className="inline-flex h-7 items-center rounded-md border border-border px-2.5 text-[12px] text-muted-foreground">Deny</span>
          </div>
        )}
      </div>

      <div className="relative mt-3 flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-border-card bg-background">
        <div className="flex items-center gap-2 border-b border-border bg-card px-3 py-2">
          <span className="flex gap-1">
            <span className="size-2 rounded-full bg-faint-2" />
            <span className="size-2 rounded-full bg-faint-2" />
            <span className="size-2 rounded-full bg-faint-2" />
          </span>
          <span className="min-w-0 flex-1 truncate rounded bg-muted px-2 py-0.5 font-mono text-[10.5px] text-muted-foreground">
            {approved ? typed(url, t, 24, 3) : ""}
            {approved && t < 34 && <Caret />}
          </span>
          <span className="shrink-0 text-[10.5px] text-faint">throwaway clone</span>
        </div>
        <div className="h-0.5">
          {approved && t < 46 && <div className="h-full bg-info" style={{ width: `${tween(t, 34, 44, 0, 100)}%` }} />}
        </div>

        {loaded && (
          <div className={`flex min-h-0 flex-1 flex-col p-3.5 ${appear}`}>
            <div className="text-[14px] font-semibold text-foreground">
              Rotate refresh tokens <span className="font-normal text-faint">#214</span>
            </div>
            <div className="mt-3 flex gap-4 border-b border-border text-[12px]">
              {["Conversation", "Commits", "Checks"].map(tab => (
                <span
                  key={tab}
                  className={`-mb-px border-b-2 pb-1.5 ${(tab === "Checks") === onChecks ? "border-foreground text-foreground" : "border-transparent text-muted-foreground"}`}
                >
                  {tab}
                </span>
              ))}
            </div>
            {onChecks ? (
              <div className="mt-3 flex flex-col gap-1.5">
                {checks.map(check => (
                  <div key={check.name} className={`flex items-center gap-2 rounded-md border border-border px-2.5 py-1.5 text-[12px] ${appear}`}>
                    {t >= check.at ? <Check size={12} className="text-success" aria-hidden="true" /> : <Spinner />}
                    <span className="flex-1 text-foreground">{check.name}</span>
                    <span className="font-mono text-[10.5px] text-faint">{t >= check.at ? "passed" : "running"}</span>
                  </div>
                ))}
                {t >= 96 && (
                  <span className={`mt-1 self-end rounded-md border px-2.5 py-1 text-[12px] text-foreground ${t >= 112 ? "border-ring bg-accent" : "border-border"} ${appear}`}>
                    Visit preview ↗
                  </span>
                )}
              </div>
            ) : (
              <div className="mt-3 flex flex-col gap-2">
                <div className="h-2 w-full rounded bg-muted/60" />
                <div className="h-2 w-5/6 rounded bg-muted/60" />
                <div className="h-2 w-2/3 rounded bg-muted/60" />
              </div>
            )}
          </div>
        )}

        {loaded && (
          <div className="pointer-events-none absolute transition-[left,top] duration-700 ease-out" style={{ left: `${cursor.x}%`, top: `${cursor.y}%` }}>
            <MousePointer2 size={16} className="fill-foreground text-background" aria-hidden="true" />
            {clicking && <span className="absolute -left-2 -top-2 size-5 animate-ping rounded-full bg-foreground/40" />}
            <span className="absolute left-4 top-4 whitespace-nowrap rounded bg-harness-claude px-1.5 py-0.5 text-[10px] font-medium text-background">Claude</span>
          </div>
        )}
      </div>

      <div className="mt-3 flex items-center gap-2 px-1 text-[11.5px]">
        <span className="size-1.5 shrink-0 rounded-full bg-success motion-safe:animate-pulse" />
        <span key={narration} className={`min-w-0 flex-1 truncate text-muted-foreground ${appear}`}>
          {narration}
        </span>
        <span className="shrink-0 rounded-md border border-border px-2 py-0.5 text-foreground">Take over</span>
      </div>
    </div>
  );
}

/* History and compaction ----------------------------------------------------------------- */

const ledger = ["message.completed", "plan.updated", "tool.started", "tool.completed", "delegation.spawned", "delegation.result", "message.completed", "tool.completed"];

/** Events stream in, the context meter fills, compaction drops it, and a restart resumes. */
export function History() {
  const { ref, t } = usePlayback<HTMLDivElement>(170);
  const shown = Math.min(ledger.length, Math.floor(t / 10) + 1);
  const compacted = t >= 92;
  const tokens = compacted ? tween(t, 92, 104, 184, 23) : tween(t, 0, 84, 41, 184);
  const restarted = t >= 120;

  return (
    <div ref={ref} className={`${frame} flex h-[440px] flex-col sm:h-[480px]`} aria-hidden="true">
      <div className="flex items-center gap-3 border-b border-border px-4 py-2.5">
        <span className="inline-flex items-center gap-1 rounded-md border border-border bg-card px-2 py-1 text-[12px] text-foreground">{"{ }"} Transcript</span>
        <span className="flex flex-1 items-center gap-2">
          <span className="h-1 flex-1 overflow-hidden rounded-full bg-muted">
            <span className={`block h-full rounded-full ${tokens > 160 ? "bg-warning" : "bg-info"}`} style={{ width: `${(tokens / 200) * 100}%` }} />
          </span>
          <span className="w-[76px] text-right font-mono text-[10.5px] tabular-nums text-muted-foreground">{Math.round(tokens)}k / 200k</span>
        </span>
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden px-4 py-2 font-mono text-[11.5px]">
        {ledger.slice(0, shown).map((kind, i) => {
          const running = i === shown - 1 && !compacted && t % 10 < 7;
          return (
            <div key={i} className={`flex items-center gap-3 border-b border-border/60 py-1.5 last:border-b-0 ${appear} ${compacted ? "opacity-55" : ""}`}>
              <span className="w-5 tabular-nums text-faint-2">{i + 1}</span>
              <span className="flex-1 text-foreground">{kind}</span>
              {running ? <Spinner size={11} /> : <span className="text-muted-foreground">completed</span>}
              <span className="tabular-nums text-faint">12:58:{String(4 + i * 3).padStart(2, "0")}</span>
            </div>
          );
        })}
      </div>

      <div className="flex flex-col gap-2 px-4 pb-4">
        {compacted && (
          <div className={`overflow-hidden rounded-lg border border-l-2 border-border border-l-info bg-card ${appear}`}>
            <div className="flex items-baseline gap-2 px-4 pt-2.5">
              <b className="text-[13px] font-semibold text-foreground">Context compacted</b>
              <small className="text-[11px] text-info">184k → 23k tokens</small>
            </div>
            <p className="px-4 pb-2.5 pt-1 text-[12px] text-muted-foreground">All 8 events above are still in history. Only the model&rsquo;s view got shorter.</p>
          </div>
        )}
        {restarted && (
          <div className={`flex items-center gap-2 rounded-lg border border-border bg-code px-3 py-2.5 ${appear}`}>
            {t < 132 ? <Spinner /> : <span className="size-1.5 rounded-full bg-success" />}
            <span className="font-mono text-[11.5px] text-foreground">{t < 132 ? "App restarted, resuming…" : "Resumed from checkpoint"}</span>
            <span className="ml-auto font-mono text-[11px] text-faint">verified boundary · turn 14</span>
          </div>
        )}
      </div>
    </div>
  );
}

/* Verification --------------------------------------------------------------------------- */

const testLines = ["policy::scope::rejects_outside_write", "session_forest::fork_keeps_parent", "worktree::reclaim_on_archive", "auth::rotation_on_write", "auth::two_readers_one_mint"];

/** Tests stream, the author's own family is refused, and a different family signs off. */
export function Verify() {
  const { ref, t } = usePlayback<HTMLDivElement>(180);
  const testsDone = t >= 48;
  const sameFamily = t >= 54;
  const sameRejected = t >= 70;
  const crossFamily = t >= 78;
  const review = "Rotation now happens on write, and the race test covers two concurrent readers. Approving.";
  const crossDone = t >= 136;

  return (
    <div ref={ref} className={`${frame} flex h-[440px] flex-col p-4 sm:h-[480px]`} aria-hidden="true">
      <div className="flex items-center gap-2 px-1 pb-3">
        <HarnessMark harness="claude" size={13} />
        <span className="text-[13px] font-medium text-foreground">Rotate refresh tokens</span>
        <span className="ml-auto font-mono text-[11px] text-faint">written by Claude Opus</span>
      </div>

      <div className="rounded-lg border border-border bg-code px-3 py-2.5 font-mono text-[11px] leading-[1.7]">
        <div className="text-foreground">$ cargo test -p bridge-core</div>
        {testLines.slice(0, Math.min(testLines.length, Math.floor(t / 8))).map(line => (
          <div key={line} className={`truncate text-muted-foreground ${appear}`}>
            test {line} ... <span className="text-success">ok</span>
          </div>
        ))}
        {testsDone ? (
          <div className={`text-foreground ${appear}`}>
            test result: <span className="text-success">ok</span>. 214 passed; 0 failed
          </div>
        ) : (
          <Caret />
        )}
      </div>

      <div className="mt-3 flex flex-col gap-2">
        {sameFamily && (
          <div className={`flex items-center gap-3 rounded-lg border bg-card px-3.5 py-2.5 ${sameRejected ? "border-destructive/40" : "border-border-card"} ${appear}`}>
            <HarnessMark harness="claude" size={13} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[13px] text-foreground">Review by Claude Code</span>
              <span className="block text-[11px] text-faint">{sameRejected ? "same model family as the author" : "reviewing…"}</span>
            </span>
            {sameRejected ? (
              <span className={`inline-flex items-center gap-1 text-[11.5px] text-destructive ${appear}`}>
                <X size={12} aria-hidden="true" /> doesn&rsquo;t count
              </span>
            ) : (
              <Spinner />
            )}
          </div>
        )}

        {crossFamily && (
          <div className={`rounded-lg border bg-card px-3.5 py-2.5 ${crossDone ? "border-success/40" : "border-border-card"} ${appear}`}>
            <div className="flex items-center gap-3">
              <HarnessMark harness="codex" size={13} />
              <span className="min-w-0 flex-1 text-[13px] text-foreground">Review by Codex</span>
              {crossDone ? (
                <span className={`inline-flex items-center gap-1 text-[11.5px] text-success ${appear}`}>
                  <Check size={12} aria-hidden="true" /> approved
                </span>
              ) : (
                <Spinner />
              )}
            </div>
            <p className="mt-1.5 pl-[25px] text-[12px] leading-5 text-body">
              {typed(review, t, 88, 2)}
              {!crossDone && t >= 88 && <Caret />}
            </p>
          </div>
        )}
      </div>

      {t >= 142 && (
        <div className={`mt-auto flex items-center gap-2 rounded-lg border border-border bg-code px-3 py-2.5 ${appear}`}>
          <span className="size-1.5 rounded-full bg-success" />
          <span className="font-mono text-[11.5px] text-foreground">Ready to merge</span>
          <span className="ml-auto font-mono text-[11px] text-faint">tests + cross-family review</span>
        </div>
      )}
    </div>
  );
}

/* Memory --------------------------------------------------------------------------------- */

/** You say it once in one chat; a different agent in a different repo already knows. */
export function Memory() {
  const { ref, t } = usePlayback<HTMLDivElement>(180);
  const say = "Use bun, never npm. And never push straight to main.";
  const ask = "Set up CI for this repo.";
  const reply = "Added a workflow that runs bun install and bun test, and opened it as a PR on ci/setup instead of pushing to main.";
  const saved = [
    { kind: "preference", text: "Use bun, never npm", at: 40 },
    { kind: "constraint", text: "Never push straight to main", at: 48 },
  ].filter(memory => t >= memory.at);
  const recalled = t >= 96 && t < 106;

  return (
    <div ref={ref} className={`${frame} flex h-[400px] flex-col gap-3 p-4 sm:h-[420px]`} aria-hidden="true">
      <div className="rounded-lg border border-border-card bg-card p-3">
        <div className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <HarnessMark harness="claude" size={12} /> Claude Code · api/
        </div>
        <div className="mt-2 ml-auto w-fit max-w-[90%] rounded-2xl border border-border bg-accent/70 px-3 py-1.5 text-[12.5px] text-foreground">
          {typed(say, t, 0, 2)}
          {t < 28 && <Caret />}
        </div>
        <div className="mt-2 flex h-5 flex-wrap gap-1.5">
          {saved.map(memory => (
            <span key={memory.kind} className={`inline-flex items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-[10.5px] text-foreground ${appear}`}>
              <Check size={10} className="text-success" aria-hidden="true" /> Saved {memory.kind}
            </span>
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-1.5 px-1">
        <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-faint">Memory</span>
        {saved.map(memory => (
          <div key={memory.kind} className={`flex items-center gap-3 rounded-md border px-2.5 py-1.5 transition-colors ${recalled ? "border-info/60" : "border-border"} ${appear}`}>
            <span className="w-[72px] shrink-0 font-mono text-[10.5px] text-faint">{memory.kind}</span>
            <span className="min-w-0 flex-1 truncate text-[12.5px] text-foreground">{memory.text}</span>
            <span className="shrink-0 font-mono text-[10px] text-faint">every repo</span>
          </div>
        ))}
      </div>

      <div className={`mt-auto rounded-lg border border-border-card bg-background p-3 transition-opacity duration-500 ${t >= 64 ? "opacity-100" : "opacity-40"}`}>
        <div className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <HarnessMark harness="codex" size={12} /> Codex · web/
          {t >= 96 && <span className={`ml-auto rounded bg-muted px-1.5 py-0.5 font-mono text-[10.5px] text-foreground ${appear}`}>2 memories used</span>}
        </div>
        {t >= 64 && (
          <div className="mt-2 ml-auto w-fit rounded-2xl border border-border bg-accent/70 px-3 py-1.5 text-[12.5px] text-foreground">
            {typed(ask, t, 64, 1)}
            {t < 88 && <Caret />}
          </div>
        )}
        {t >= 98 && (
          <p className="mt-2 text-[12.5px] leading-5 text-body">
            {typed(reply, t, 100, 3)}
            {t < 140 && <Caret />}
          </p>
        )}
      </div>
    </div>
  );
}

/* Usage ---------------------------------------------------------------------------------- */

/* The daily-cost series, one per harness. Fixed values so the curve never shifts. */
const series = [
  { id: "claude", stroke: "stroke-harness-claude", fill: "fill-harness-claude/20", points: [6, 9, 22, 74, 38, 30, 52, 28, 18, 41, 15, 12, 9, 26, 58, 22, 16, 34, 12, 8, 18, 46, 68, 44, 30, 55, 38, 62, 24, 12] },
  { id: "codex", stroke: "stroke-harness-codex", fill: "fill-harness-codex/20", points: [2, 3, 4, 6, 5, 4, 8, 6, 3, 5, 4, 3, 2, 4, 6, 5, 4, 22, 14, 6, 4, 8, 12, 9, 6, 18, 11, 24, 9, 5] },
  { id: "opencode", stroke: "stroke-harness-opencode", fill: "fill-harness-opencode/20", points: [1, 2, 2, 3, 4, 14, 8, 4, 2, 3, 2, 2, 1, 3, 4, 3, 2, 5, 4, 3, 2, 4, 16, 7, 4, 9, 6, 13, 22, 6] },
];

const W = 560;
const H = 190;
const peak = 80;

function path(points: number[], close: boolean) {
  const step = W / (points.length - 1);
  const y = (v: number) => H - (v / peak) * H;
  const line = points.map((v, i) => `${i === 0 ? "M" : "L"}${(i * step).toFixed(1)} ${y(v).toFixed(1)}`).join(" ");
  return close ? `${line} L${W} ${H} L0 ${H} Z` : line;
}

const money = (n: number) => `$${n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;

/** The usage screen running live: the curves draw, then the totals tick up as requests land. */
export function Usage() {
  const { ref, t } = usePlayback<HTMLDivElement>(160);
  const drawn = tween(t, 0, 40, 0, 1);
  const live = Math.max(0, t - 40);
  const scale = tween(t, 0, 40, 0.94, 1);
  const harnesses = [
    { id: "claude", name: "Claude", cost: 5535.39 * scale + live * 0.41, note: "76.5% of cost" },
    { id: "codex", name: "Codex", cost: 1092.75 * scale + live * 0.07, note: "15.1% of cost" },
    { id: "opencode", name: "OpenCode", cost: 609.39 * scale + live * 0.03, note: "8.4% of cost" },
  ];
  const total = harnesses.reduce((sum, harness) => sum + harness.cost, 0);
  const stats = [
    ["Processed tokens", `${(10.4 + live * 0.0004).toFixed(2)}B`],
    ["Cached input", `${(10.1 + live * 0.0004).toFixed(2)}B`],
    ["Output", `${(40.6 + live * 0.01).toFixed(1)}M`],
    ["Cache savings", money(tween(t, 0, 40, 41000, 43267.54) + live * 1.9)],
  ];

  return (
    <div ref={ref} className={`${frame} p-4`} aria-hidden="true">
      <div className="grid gap-3 lg:grid-cols-[minmax(0,210px)_minmax(0,1fr)]">
        <div className="rounded-lg border border-border-card bg-card p-3.5">
          <div className="font-heading text-[26px] leading-none tracking-[-0.02em] text-foreground tabular-nums">{money(total)}</div>
          <p className="mt-1.5 flex items-center gap-1.5 text-[10.5px] text-muted-foreground">
            <span className="size-1.5 rounded-full bg-success motion-safe:animate-pulse" />
            {(61176 + live * 3).toLocaleString("en-US")} requests · live
          </p>
          <div className="mt-3 flex flex-col gap-2.5">
            {harnesses.map(harness => (
              <div key={harness.id}>
                <div className="flex items-center gap-1.5 text-[12px]">
                  <HarnessMark harness={harness.id} size={11} />
                  <span className="text-foreground">{harness.name}</span>
                  <span className="ml-auto tabular-nums text-foreground">{money(harness.cost)}</span>
                </div>
                <p className="mt-0.5 pl-4 text-[10.5px] text-muted-foreground">{harness.note}</p>
              </div>
            ))}
          </div>
        </div>

        <div className="rounded-lg border border-border-card bg-card p-3.5">
          <div className="text-[12px] font-medium text-foreground">Daily cost</div>
          <svg viewBox={`0 0 ${W} ${H}`} className="mt-2 h-[140px] w-full" fill="none" preserveAspectRatio="none">
            {[0, 0.5, 1].map(at => (
              <line key={at} x1="0" x2={W} y1={H * at} y2={H * at} className="stroke-border" strokeWidth="1" vectorEffect="non-scaling-stroke" />
            ))}
            {series.map(s => (
              <g key={s.id}>
                <path d={path(s.points, true)} className={s.fill} opacity={drawn} />
                <path d={path(s.points, false)} className={s.stroke} strokeDasharray={4000} strokeDashoffset={4000 * (1 - drawn)} strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
              </g>
            ))}
          </svg>
          <div className="mt-1.5 flex justify-between font-mono text-[10px] text-faint">
            <span>Aug 13</span>
            <span>Aug 27</span>
            <span>Today</span>
          </div>
        </div>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-2 lg:grid-cols-4">
        {stats.map(([label, value]) => (
          <div key={label} className="rounded-lg border border-border-card bg-card px-3 py-2">
            <div className="truncate text-[10px] text-muted-foreground">{label}</div>
            <div className="mt-0.5 text-[14px] tabular-nums text-foreground">{value}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
