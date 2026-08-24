import { useCallback, useEffect, useRef, useState } from "react";
import {
  Activity,
  ArrowUp,
  Ban,
  Braces,
  Check,
  ChevronRight,
  CircleAlert,
  Clock,
  Code2,
  FileDiff,
  Globe,
  Hand,
  Maximize2,
  MessageSquareText,
  Minimize2,
  Moon,
  MoreHorizontal,
  PanelRight,
  Plus,
  Quote,
  RotateCcw,
  Search,
  Sun,
  TerminalSquare,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { useThemePreference } from "../theme";

// A non-functional design mockup of the right-hand dock. Nothing here talks to
// the runtime: every shell, diff, event and task below is a hand-written
// fixture, and the only real behaviour is layout — resize, switch, expand,
// collapse — because a dock cannot be judged from a screenshot.

type PaneId = "terminal" | "changes" | "code" | "browser" | "transcript" | "tasks";

type PaneBadge = { kind: "count"; value: number } | { kind: "state"; tone: "running" | "attention" | "failed" };

type PaneSpec = {
  id: PaneId;
  label: string;
  icon: LucideIcon;
  /** Panes that inspect a checkout are meaningless in a chat with no repository. */
  needsRepo: boolean;
  badge?: PaneBadge;
};

const PANES: PaneSpec[] = [
  { id: "terminal", label: "Terminal", icon: TerminalSquare, needsRepo: true, badge: { kind: "state", tone: "running" } },
  { id: "changes", label: "Changes", icon: FileDiff, needsRepo: true, badge: { kind: "count", value: 12 } },
  { id: "code", label: "Code", icon: Code2, needsRepo: true },
  { id: "browser", label: "Browser", icon: Globe, needsRepo: false, badge: { kind: "state", tone: "attention" } },
  { id: "transcript", label: "Transcript", icon: Braces, needsRepo: false },
  { id: "tasks", label: "Tasks", icon: Activity, needsRepo: false, badge: { kind: "state", tone: "failed" } },
];

const MIN_DOCK = 320;
const MAX_DOCK = 760;
const MIN_CONVERSATION = 440;
const DEFAULT_DOCK = 440;

export function DockPreview() {
  const { resolved, setPreference } = useThemePreference();
  const [hasRepo, setHasRepo] = useState(true);
  const [dockOpen, setDockOpen] = useState(true);
  const [expanded, setExpanded] = useState(false);
  const [activePane, setActivePane] = useState<PaneId>("changes");
  const [width, setWidth] = useState(DEFAULT_DOCK);
  const [dragging, setDragging] = useState(false);
  const [narrow, setNarrow] = useState(false);
  const sectionRef = useRef<HTMLDivElement>(null);

  const clampWidth = useCallback((next: number) => {
    const available = sectionRef.current?.clientWidth ?? 1200;
    return Math.min(Math.min(MAX_DOCK, available - MIN_CONVERSATION), Math.max(MIN_DOCK, next));
  }, []);

  // Below the point where both sides can hold their minimum, a split is worse
  // than a sheet: the dock slides over the conversation instead of starving it.
  useEffect(() => {
    const element = sectionRef.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      const available = entry.contentRect.width;
      setNarrow(available < MIN_CONVERSATION + MIN_DOCK);
      setWidth(current => Math.max(MIN_DOCK, Math.min(current, Math.max(MIN_DOCK, available - MIN_CONVERSATION))));
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const originX = event.clientX;
    const originWidth = width;
    setDragging(true);
    const move = (pointer: PointerEvent) => setWidth(clampWidth(originWidth + (originX - pointer.clientX)));
    const release = () => {
      setDragging(false);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", release);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", release);
  };

  // ⌥⌘0 for the dock, ⌥⌘1–6 for its panes, ⌥⌘↩ to expand. ⌥⌘F stays the
  // conversation's fullscreen, and none of these are taken by macOS.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.altKey || !(event.metaKey || event.ctrlKey)) return;
      if (event.key === "0") { event.preventDefault(); setDockOpen(value => !value); return; }
      if (event.key === "Enter") { event.preventDefault(); setExpanded(value => !value); return; }
      const index = Number(event.key);
      if (Number.isInteger(index) && index >= 1 && index <= PANES.length) {
        event.preventDefault();
        setActivePane(PANES[index - 1].id);
        setDockOpen(true);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const openPane = (id: PaneId) => { setActivePane(id); setDockOpen(true); };
  const available = (pane: PaneSpec) => !pane.needsRepo || hasRepo;

  return (
    <div className="relative flex h-[100dvh] overflow-hidden bg-background text-foreground">
      {!expanded && <PreviewSidebar hasRepo={hasRepo} />}

      <main className="relative z-10 flex min-w-0 flex-1 flex-col overflow-hidden">
        <PreviewToolbar
          title={hasRepo ? "Right-hand dock" : "Scratch questions"}
          model={hasRepo ? "Claude Opus 4.6" : undefined}
          dockOpen={dockOpen}
          expanded={expanded}
          onToggleDock={() => setDockOpen(value => !value)}
        />

        <section ref={sectionRef} className="relative flex min-h-0 flex-1 overflow-hidden">
          {!expanded && (
            <div className="relative flex min-w-0 flex-1 flex-col">
              <PreviewConversation hasRepo={hasRepo} onOpenPane={openPane} />
            </div>
          )}

          {dockOpen && !expanded && !narrow && (
            <div
              role="separator"
              aria-orientation="vertical"
              aria-label="Resize dock"
              onPointerDown={startResize}
              onDoubleClick={() => setWidth(DEFAULT_DOCK)}
              title="Drag to resize · double-click to reset"
              className="group relative z-20 -mr-1.5 w-3 shrink-0 cursor-col-resize"
            >
              <span
                className={cn(
                  "absolute inset-y-0 left-1.5 w-px transition-colors",
                  dragging ? "bg-ring" : "bg-border group-hover:bg-ring/60",
                )}
              />
            </div>
          )}

          {dockOpen ? (
            <>
            {narrow && !expanded && (
              <button
                type="button"
                aria-label="Close dock"
                onClick={() => setDockOpen(false)}
                className="absolute inset-0 z-20 bg-scrim"
              />
            )}
            <div
              className={cn(
                "flex min-w-0 flex-col border-l border-border bg-sidebar",
                expanded ? "flex-1" : narrow ? "absolute inset-y-0 right-0 z-30 shadow-2xl" : "shrink-0",
              )}
              style={expanded ? undefined : { width }}
            >
              <DockSwitcher
                activePane={activePane}
                expanded={expanded}
                hasRepo={hasRepo}
                onSelect={setActivePane}
                onToggleExpand={() => setExpanded(value => !value)}
                onClose={() => { setExpanded(false); setDockOpen(false); }}
              />
              <div className="min-h-0 flex-1 overflow-hidden">
                {PANES.map(pane => (
                  <div key={pane.id} className={cn("h-full", activePane !== pane.id && "hidden")}>
                    {available(pane) ? <PaneBody id={pane.id} expanded={expanded} /> : <UnavailablePane pane={pane} />}
                  </div>
                ))}
              </div>
            </div>
            </>
          ) : (
            <CollapsedRail hasRepo={hasRepo} activePane={activePane} onOpenPane={openPane} />
          )}
        </section>
      </main>

      <PreviewControls
        dark={resolved === "dark"}
        hasRepo={hasRepo}
        onToggleTheme={() => setPreference(resolved === "dark" ? "light" : "dark")}
        onToggleRepo={() => setHasRepo(value => !value)}
        onReset={() => { setWidth(DEFAULT_DOCK); setDockOpen(true); setExpanded(false); setActivePane("changes"); }}
      />
    </div>
  );
}

/* ── Chrome ───────────────────────────────────────────────────────────────── */

function PreviewToolbar({ title, model, dockOpen, expanded, onToggleDock }: {
  title: string;
  model?: string;
  dockOpen: boolean;
  expanded: boolean;
  onToggleDock: () => void;
}) {
  return (
    <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border pl-4 pr-2 sm:pl-6">
      <h1 className="m-0 min-w-0 flex-1 truncate font-display text-[14px] font-semibold leading-none tracking-[-0.014em] text-foreground">
        {title}
      </h1>
      {model && <p className="hidden shrink-0 truncate text-[11px] text-muted-foreground lg:block">{model}</p>}
      <button
        type="button"
        onClick={onToggleDock}
        aria-pressed={dockOpen}
        title="Toggle dock  ⌥⌘0"
        className={cn(
          "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors",
          dockOpen && !expanded ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
        )}
      >
        <PanelRight size={15} strokeWidth={1.8} aria-hidden="true" />
      </button>
      <button
        type="button"
        title="Session actions"
        className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <MoreHorizontal size={15} strokeWidth={1.8} aria-hidden="true" />
      </button>
    </div>
  );
}

function PreviewSidebar({ hasRepo }: { hasRepo: boolean }) {
  const chats: { name: string; tone: string; active?: boolean }[] = [
    { name: "Right-hand dock", tone: "bg-success", active: hasRepo },
    { name: "Session forest replay", tone: "bg-muted-foreground/25" },
    { name: "Worktree adoption bug", tone: "bg-warning" },
    { name: "Scratch questions", tone: "bg-muted-foreground/25", active: !hasRepo },
  ];
  return (
    <aside className="hidden w-[248px] shrink-0 flex-col border-r border-sidebar-border bg-sidebar px-2 py-3 md:flex">
      <p className="px-2 pb-4 font-display text-[14px] font-semibold tracking-[-0.012em] text-foreground">bridge</p>
      <div className="relative mb-3">
        <Search size={13} strokeWidth={1.7} aria-hidden="true" className="pointer-events-none absolute left-2 top-2 text-muted-foreground" />
        <div className="h-7 w-full rounded-lg border border-border bg-background pl-7 pr-2 text-[13px] leading-7 text-muted-foreground/70">Search</div>
      </div>
      <div className="u-segmented mb-3 w-full">
        <span className="u-segmented-item flex-1 text-center" data-active={!hasRepo}>Work</span>
        <span className="u-segmented-item flex-1 text-center" data-active={hasRepo}>Code</span>
      </div>
      <div className="flex h-6 items-center px-2">
        <span className="text-[11px] font-semibold text-muted-foreground">Today</span>
      </div>
      <div className="flex flex-col gap-px">
        {chats.map(chat => (
          <div
            key={chat.name}
            className={cn(
              "flex h-7 items-center gap-2 rounded-md px-2 text-[13px] tracking-[-0.006em]",
              chat.active ? "bg-accent font-medium text-foreground" : "text-muted-foreground",
            )}
          >
            <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", chat.tone)} />
            <span className="min-w-0 flex-1 truncate">{chat.name}</span>
          </div>
        ))}
      </div>
    </aside>
  );
}

/* ── Conversation ─────────────────────────────────────────────────────────── */

function PreviewConversation({ hasRepo, onOpenPane }: { hasRepo: boolean; onOpenPane: (id: PaneId) => void }) {
  return (
    <>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-2xl flex-col gap-7 px-6 py-8">
          <div className="flex justify-end">
            <p className="u-surface max-w-[85%] rounded-2xl px-4 py-2.5 text-[14px] leading-relaxed text-foreground">
              opening the terminal hides the whole turn. can it sit beside the conversation instead?
            </p>
          </div>

          <div className="md">
            <p>
              It can. The panels already exist — they are just rendered as full-area overlays. Moving them into a
              trailing dock keeps the conversation on screen while you inspect.
            </p>
            <p>
              I put the split shell in{" "}
              <button type="button" onClick={() => onOpenPane("code")} className="rounded px-1 font-mono text-[0.85em] text-[var(--color-ring)] underline underline-offset-2 hover:bg-accent">
                SessionDock.tsx
              </button>{" "}
              and the width and pane persistence in{" "}
              <button type="button" onClick={() => onOpenPane("code")} className="rounded px-1 font-mono text-[0.85em] text-[var(--color-ring)] underline underline-offset-2 hover:bg-accent">
                dockLayout.ts
              </button>
              , keyed per workspace.
            </p>
          </div>

          <button
            type="button"
            onClick={() => onOpenPane("changes")}
            className="u-surface flex w-full items-center gap-2.5 rounded-xl px-3 py-2 text-left transition-colors hover:bg-accent"
          >
            <FileDiff size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">Edited 12 files</span>
            <span className="shrink-0 font-mono text-[11px]">
              <b className="font-medium text-success">+162</b> <b className="font-medium text-destructive">−38</b>
            </span>
            <ChevronRight size={13} className="shrink-0 text-muted-foreground/60" aria-hidden="true" />
          </button>

          <div className="md">
            <p>
              Tests are green — 23 across the layout reducer and the persistence round-trip. The divider clamps both
              sides, so the conversation never drops below a readable width.
            </p>
            <p className="flex items-center gap-2 text-muted-foreground">
              <span className="h-1.5 w-1.5 rounded-full bg-foreground/40" />
              <span className="thinking-shimmer h-3 w-40 rounded bg-clip-text text-transparent">writing the collapsed rail</span>
            </p>
          </div>
        </div>
      </div>

      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-16 bg-gradient-to-t from-background to-transparent" />

      <div className="relative z-10 flex-none pb-5">
        {hasRepo && (
          <div className="mx-auto mb-2 flex max-w-2xl justify-center px-6">
            <div className="u-surface inline-flex h-[30px] items-center gap-2 rounded-full px-3.5 text-xs text-muted-foreground">
              <FileDiff size={12} aria-hidden="true" />
              <span>12 files</span>
              <em className="font-mono text-[11px] not-italic">
                <b className="text-success">+162</b> <b className="text-destructive">−38</b>
              </em>
            </div>
          </div>
        )}
        <div className="mx-auto max-w-2xl px-6">
          <div className="u-surface flex items-center gap-2 rounded-[1.4rem] px-3 py-2.5">
            <span className="inline-flex h-8 w-8 items-center justify-center rounded-full text-muted-foreground">
              <Plus size={16} strokeWidth={1.8} aria-hidden="true" />
            </span>
            <span className="min-w-0 flex-1 truncate text-[14px] text-muted-foreground/70">Message…</span>
            <span className="inline-flex h-8 items-center rounded-full px-2.5 text-[13px] text-foreground/75">Opus</span>
            <span className="inline-flex h-8 w-8 items-center justify-center rounded-full bg-primary text-primary-foreground">
              <ArrowUp size={15} strokeWidth={2.2} aria-hidden="true" />
            </span>
          </div>
        </div>
      </div>
    </>
  );
}

/* ── Dock chrome ──────────────────────────────────────────────────────────── */

const STATE_TONE: Record<"running" | "attention" | "failed", string> = {
  running: "bg-success",
  attention: "bg-warning",
  failed: "bg-destructive",
};

/** A count rides inline because it is content; a state rides as a corner dot
 *  because it is a condition — inline, the two read as separate controls. */
function StateDot({ tone, className }: { tone: "running" | "attention" | "failed"; className?: string }) {
  return (
    <span
      className={cn(
        "pointer-events-none absolute right-0 top-0 h-[5px] w-[5px] rounded-full",
        STATE_TONE[tone],
        tone === "attention" && "mission-live-accent",
        className,
      )}
    />
  );
}

function DockSwitcher({ activePane, expanded, hasRepo, onSelect, onToggleExpand, onClose }: {
  activePane: PaneId;
  expanded: boolean;
  hasRepo: boolean;
  onSelect: (id: PaneId) => void;
  onToggleExpand: () => void;
  onClose: () => void;
}) {
  return (
    <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-2">
      <div role="tablist" aria-label="Dock panes" className="flex h-7 min-w-0 items-center gap-0.5 rounded-lg border border-border bg-muted p-0.5">
        {PANES.map((pane, index) => {
          const active = pane.id === activePane;
          const dimmed = pane.needsRepo && !hasRepo;
          const Icon = pane.icon;
          return (
            <button
              key={pane.id}
              type="button"
              role="tab"
              aria-selected={active}
              title={`${pane.label}  ⌥⌘${index + 1}${dimmed ? " — needs a repository" : ""}`}
              onClick={() => onSelect(pane.id)}
              className={cn(
                "relative flex h-6 shrink-0 items-center gap-1.5 rounded-md px-1.5 text-[11px] font-medium transition-colors",
                active ? "bg-card text-foreground" : "text-muted-foreground hover:text-foreground",
                dimmed && !active && "opacity-40",
              )}
            >
              <Icon size={13} strokeWidth={1.7} aria-hidden="true" />
              {active && <span className="pr-0.5">{pane.label}</span>}
              {pane.badge?.kind === "count" && !dimmed && (
                <span className="rounded-full bg-accent px-1 font-mono text-[10px] leading-4 text-muted-foreground">{pane.badge.value}</span>
              )}
              {pane.badge?.kind === "state" && !dimmed && <StateDot tone={pane.badge.tone} />}
            </button>
          );
        })}
      </div>

      <span className="ml-auto" />

      <button
        type="button"
        onClick={onToggleExpand}
        title={expanded ? "Restore  ⌥⌘↩" : "Expand  ⌥⌘↩"}
        className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        {expanded
          ? <Minimize2 size={14} strokeWidth={1.8} aria-hidden="true" />
          : <Maximize2 size={14} strokeWidth={1.8} aria-hidden="true" />}
      </button>
      <button
        type="button"
        onClick={onClose}
        title="Close dock  ⌥⌘0"
        className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <X size={14} strokeWidth={1.8} aria-hidden="true" />
      </button>
    </div>
  );
}

/** Collapsed is a state, not a disappearance: the rail keeps every pane one
 *  click away and keeps reporting the ones that need attention. */
function CollapsedRail({ hasRepo, activePane, onOpenPane }: {
  hasRepo: boolean;
  activePane: PaneId;
  onOpenPane: (id: PaneId) => void;
}) {
  return (
    <div className="flex w-11 shrink-0 flex-col items-center gap-1 border-l border-border bg-sidebar py-2">
      {PANES.map((pane, index) => {
        const dimmed = pane.needsRepo && !hasRepo;
        const Icon = pane.icon;
        return (
          <button
            key={pane.id}
            type="button"
            onClick={() => onOpenPane(pane.id)}
            title={`${pane.label}  ⌥⌘${index + 1}`}
            className={cn(
              "relative inline-flex h-8 w-8 items-center justify-center rounded-md transition-colors",
              pane.id === activePane ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
              dimmed && "opacity-40",
            )}
          >
            <Icon size={15} strokeWidth={1.7} aria-hidden="true" />
            {pane.badge?.kind === "count" && !dimmed && (
              <span className="pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-muted-foreground/70" />
            )}
            {pane.badge?.kind === "state" && !dimmed && <StateDot tone={pane.badge.tone} className="right-1 top-1" />}
          </button>
        );
      })}
    </div>
  );
}

function UnavailablePane({ pane }: { pane: PaneSpec }) {
  const Icon = pane.icon;
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 px-8 text-center">
      <Icon size={20} strokeWidth={1.5} className="text-muted-foreground/50" aria-hidden="true" />
      <p className="max-w-[34ch] text-[12.5px] leading-relaxed text-muted-foreground">
        {pane.label} needs a repository. This is a direct chat, so there is no worktree to {pane.id === "terminal" ? "run a shell in" : pane.id === "changes" ? "diff" : "read files from"}.
      </p>
      <button type="button" className="u-surface rounded-lg px-2.5 py-1 text-[11.5px] text-foreground transition-colors hover:bg-accent">
        Connect a folder
      </button>
    </div>
  );
}

/* ── Pane bodies ──────────────────────────────────────────────────────────── */

function PaneBody({ id, expanded }: { id: PaneId; expanded: boolean }) {
  if (id === "terminal") return <TerminalMock />;
  if (id === "changes") return <ChangesMock expanded={expanded} />;
  if (id === "code") return <CodeMock />;
  if (id === "browser") return <BrowserMock />;
  if (id === "transcript") return <TranscriptMock />;
  return <TasksMock />;
}

function PaneFooter({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-7 shrink-0 items-center gap-2 border-t border-border px-2.5 font-mono text-[10.5px] text-muted-foreground">
      {children}
    </div>
  );
}

function TerminalMock() {
  const shells = [
    { name: "dev", state: "running" as const },
    { name: "vitest", state: "idle" as const },
    { name: "zsh", state: "idle" as const },
  ];
  return (
    <div className="flex h-full flex-col bg-[var(--color-code)]">
      <div className="flex h-8 shrink-0 items-center gap-0.5 border-b border-border px-1.5">
        {shells.map((shell, index) => (
          <span
            key={shell.name}
            className={cn(
              "flex h-6 items-center gap-1.5 rounded-md px-2 text-[11px]",
              index === 1 ? "bg-card text-foreground" : "text-muted-foreground",
            )}
          >
            {shell.state === "running" && <span className="h-1.5 w-1.5 rounded-full bg-success" />}
            {shell.name}
          </span>
        ))}
        <button type="button" className="ml-1 inline-flex h-6 w-6 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground" title="New shell">
          <Plus size={13} strokeWidth={1.8} aria-hidden="true" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-3 py-2.5 font-mono text-[11.5px] leading-[1.65]">
        <p className="text-[var(--syn-comment)]">.worktrees/right-hand-dock</p>
        <p><span className="text-[var(--syn-string)]">❯</span> bun run test</p>
        <p className="text-muted-foreground"> RUN  v2.1.9</p>
        <p><span className="text-[var(--color-success)]">✓</span> src/components/SessionDock.test.tsx <span className="text-muted-foreground">(14)</span></p>
        <p><span className="text-[var(--color-success)]">✓</span> src/dockLayout.test.ts <span className="text-muted-foreground">(9)</span></p>
        <p className="mt-1.5"> Test Files  <span className="text-[var(--color-success)]">2 passed</span> (2)</p>
        <p>      Tests  <span className="text-[var(--color-success)]">23 passed</span> (23)</p>
        <p className="mt-1.5"><span className="text-[var(--syn-string)]">❯</span> <span className="inline-block h-[1.1em] w-[0.55em] translate-y-[0.15em] bg-foreground/70" /></p>
      </div>
      <PaneFooter>
        <span className="truncate">~/bridge-harness/.worktrees/right-hand-dock</span>
        <span className="ml-auto shrink-0">zsh · 2000 lines</span>
      </PaneFooter>
    </div>
  );
}

function ChangesMock({ expanded }: { expanded: boolean }) {
  const files = [
    { path: "src/components/SessionDock.tsx", add: 214, del: 0, tone: "text-success", state: "new" },
    { path: "src/dockLayout.ts", add: 96, del: 0, tone: "text-success", state: "new" },
    { path: "src/App.tsx", add: 38, del: 64, tone: "", state: "open" },
    { path: "src/components/SessionToolbar.tsx", add: 9, del: 21, tone: "", state: "" },
    { path: "src/index.css", add: 12, del: 0, tone: "", state: "" },
  ];
  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-x-2 gap-y-1 border-b border-border px-2.5 py-2 text-[11px]">
        <span className="font-mono text-foreground">feat/right-hand-dock</span>
        <span className="text-muted-foreground/60">←</span>
        <span className="font-mono text-muted-foreground">main</span>
        <span className="ml-auto font-mono">
          <b className="font-medium text-success">+162</b> <b className="font-medium text-destructive">−38</b>
        </span>
        <span className="w-full text-muted-foreground/70">7/12 viewed</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {files.map(file => {
          const open = file.state === "open";
          const dir = file.path.slice(0, file.path.lastIndexOf("/") + 1);
          const base = file.path.slice(dir.length);
          return (
            <div key={file.path} className="border-b border-border">
              <div className="group flex h-8 items-center gap-2 px-2.5 transition-colors hover:bg-accent">
                <ChevronRight size={12} className={cn("shrink-0 text-muted-foreground/60", open && "rotate-90")} aria-hidden="true" />
                <span className="min-w-0 flex-1 truncate text-[11.5px]">
                  <span className="text-muted-foreground">{dir}</span>
                  <span className={cn("text-foreground", file.tone)}>{base}</span>
                </span>
                <button
                  type="button"
                  title="Reference in the composer"
                  className="hidden h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground group-hover:inline-flex"
                >
                  <Quote size={11} strokeWidth={1.8} aria-hidden="true" />
                </button>
                <span className="shrink-0 font-mono text-[10.5px]">
                  <b className="font-medium text-success">+{file.add}</b> <b className="font-medium text-destructive">−{file.del}</b>
                </span>
              </div>
              {open && (
                <div className="overflow-x-auto bg-[var(--color-code)] font-mono text-[11px] leading-[1.7]">
                  <p className="px-2.5 text-[var(--syn-comment)]">@@ -1293,8 +1293,4 @@ function App() {'{'}</p>
                  <DiffLine tone="del">{"    {visitedTabs.has(\"changes\") && <div className={cn(\"absolute inset-0\", …)}>"}</DiffLine>
                  <DiffLine tone="del">{"      <ChangesPanel workspace={workspace} />"}</DiffLine>
                  <DiffLine tone="del">{"    </div>}"}</DiffLine>
                  <DiffLine tone="add">{"    <SessionDock workspace={workspace} layout={dock} onLayout={setDock}>"}</DiffLine>
                  <DiffLine tone="add">{"      {pane => <DockPane id={pane} workspace={workspace} />}"}</DiffLine>
                  <DiffLine tone="add">{"    </SessionDock>"}</DiffLine>
                  {expanded && (
                    <>
                      <p className="mt-1 px-2.5 text-[var(--syn-comment)]">@@ -115,3 +115,2 @@</p>
                      <DiffLine tone="del">{"  const [activeTab, setActiveTab] = useState<\"agent\" | \"changes\" | …>(\"agent\");"}</DiffLine>
                      <DiffLine tone="add">{"  const [dock, setDock] = useDockLayout(workspace?.id);"}</DiffLine>
                    </>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
      <PaneFooter>
        <span>uncommitted vs HEAD</span>
        <span className="ml-auto flex items-center gap-1"><Check size={11} aria-hidden="true" /> live</span>
      </PaneFooter>
    </div>
  );
}

function DiffLine({ tone, children }: { tone: "add" | "del"; children: React.ReactNode }) {
  return (
    <p
      className={cn(
        "whitespace-pre px-2.5",
        tone === "add"
          ? "bg-[color-mix(in_srgb,var(--color-success)_12%,transparent)] text-foreground"
          : "bg-[color-mix(in_srgb,var(--color-destructive)_11%,transparent)] text-foreground",
      )}
    >
      <span className={tone === "add" ? "text-success" : "text-destructive"}>{tone === "add" ? "+" : "−"}</span>
      {children}
    </p>
  );
}

function CodeMock() {
  const lines: { no: number; parts: { text: string; tone?: string }[] }[] = [
    { no: 38, parts: [{ text: "export function ", tone: "keyword" }, { text: "SessionDock", tone: "function" }, { text: "({ layout, onLayout, children }: Props) {" }] },
    { no: 39, parts: [{ text: "  const ", tone: "keyword" }, { text: "[dragging, setDragging] = " }, { text: "useState", tone: "function" }, { text: "(" }, { text: "false", tone: "number" }, { text: ");" }] },
    { no: 40, parts: [{ text: "  const ", tone: "keyword" }, { text: "clamp = (next: " }, { text: "number", tone: "type" }, { text: ") =>" }] },
    { no: 41, parts: [{ text: "    Math.min(MAX, Math.max(MIN, next));" }] },
    { no: 42, parts: [] },
    { no: 43, parts: [{ text: "  // The divider owns both minimums: a dock that", tone: "comment" }] },
    { no: 44, parts: [{ text: "  // starves the conversation is the bug we replaced.", tone: "comment" }] },
    { no: 45, parts: [{ text: "  return ", tone: "keyword" }, { text: "(" }] },
    { no: 46, parts: [{ text: "    <" }, { text: "aside", tone: "tag" }, { text: " className=" }, { text: "\"flex min-w-0 flex-col\"", tone: "string" }, { text: ">" }] },
    { no: 47, parts: [{ text: "      {children(layout.pane)}" }] },
    { no: 48, parts: [{ text: "    </" }, { text: "aside", tone: "tag" }, { text: ">" }] },
  ];
  const toneClass = (tone?: string) => tone
    ? `text-[var(--syn-${tone === "function" ? "function" : tone})]`
    : "text-[var(--code-foreground)]";
  return (
    <div className="flex h-full flex-col bg-[var(--color-code)]">
      <div className="flex h-8 shrink-0 items-center gap-0.5 border-b border-border px-1.5">
        <span className="flex h-6 items-center gap-1.5 rounded-md bg-card px-2 text-[11px] text-foreground">SessionDock.tsx</span>
        <span className="flex h-6 items-center gap-1.5 rounded-md px-2 text-[11px] text-muted-foreground">
          dockLayout.ts
          <span className="h-1.5 w-1.5 rounded-full bg-foreground/50" title="Unsaved" />
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-auto py-2 font-mono text-[11.5px] leading-[1.65]">
        {lines.map(line => (
          <p key={line.no} className={cn("flex whitespace-pre", line.no === 43 && "bg-[var(--color-code-highlight)]")}>
            <span className="w-9 shrink-0 pr-2.5 text-right text-[10.5px] text-muted-foreground/50">{line.no}</span>
            <span className="min-w-0">
              {line.parts.map((part, index) => <span key={index} className={toneClass(part.tone)}>{part.text}</span>)}
            </span>
          </p>
        ))}
      </div>
      <PaneFooter>
        <span>Ln 43, Col 5</span>
        <span className="ml-auto flex items-center gap-1.5">
          <span className="h-1.5 w-1.5 rounded-full bg-foreground/50" /> 1 unsaved
        </span>
      </PaneFooter>
    </div>
  );
}

function BrowserMock() {
  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-border px-2.5 py-2">
        <span className="inline-flex items-center gap-1.5 rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 text-[10.5px] font-medium text-warning">
          <Hand size={10} strokeWidth={2} aria-hidden="true" /> Waiting for you
        </span>
        <span className="ml-auto font-mono text-[10px] text-muted-foreground">lease 4:12</span>
      </div>
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border px-2.5">
        <Globe size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground">app.example.com/checkout</span>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden p-2.5">
        <div className="flex h-full flex-col gap-2.5 rounded-lg border border-border bg-card p-3">
          <div className="h-2.5 w-24 rounded bg-foreground/15" />
          <div className="h-2 w-full rounded bg-foreground/8" />
          <div className="h-2 w-4/5 rounded bg-foreground/8" />
          <div className="mt-1 rounded-md border-2 border-dashed border-warning/60 bg-warning/5 p-2.5">
            <div className="h-2 w-20 rounded bg-foreground/15" />
            <p className="mt-1.5 text-[10.5px] leading-snug text-warning">Agent paused: card details are yours to enter</p>
          </div>
          <div className="h-2 w-2/3 rounded bg-foreground/8" />
          <div className="mt-auto h-7 w-28 rounded-md bg-foreground/12" />
        </div>
      </div>
      <PaneFooter>
        <span className="flex items-center gap-1"><Ban size={11} aria-hidden="true" /> redacting 3 fields</span>
        <span className="ml-auto">attached · reading</span>
      </PaneFooter>
    </div>
  );
}

function TranscriptMock() {
  const events = [
    { seq: 1841, kind: "turn.started", source: "claude", time: "14:22:07", tone: "text-[var(--syn-keyword)]" },
    { seq: 1842, kind: "message.delta", source: "claude", time: "14:22:07", tone: "text-muted-foreground" },
    { seq: 1843, kind: "tool.call", source: "claude", time: "14:22:09", tone: "text-[var(--syn-function)]" },
    { seq: 1844, kind: "file.changed", source: "core", time: "14:22:11", tone: "text-[var(--color-success)]" },
    { seq: 1845, kind: "approval.requested", source: "policy", time: "14:22:12", tone: "text-[var(--color-warning)]" },
    { seq: 1846, kind: "approval.granted", source: "user", time: "14:22:19", tone: "text-[var(--color-warning)]" },
    { seq: 1847, kind: "tool.result", source: "claude", time: "14:22:20", tone: "text-[var(--syn-function)]" },
    { seq: 1848, kind: "usage.reported", source: "claude", time: "14:22:24", tone: "text-muted-foreground" },
  ];
  return (
    <div className="flex h-full flex-col">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border px-2.5 text-[11px]">
        <span className="u-segmented h-6">
          <span className="u-segmented-item" data-active="true">Stream</span>
          <span className="u-segmented-item">Branches</span>
        </span>
        <span className="ml-auto font-mono text-[10px] text-muted-foreground">tailing</span>
      </div>
      <div className="min-h-0 flex-1 overflow-auto py-1 font-mono text-[10.5px] leading-[1.9]">
        {events.map(event => (
          <p key={event.seq} className={cn("flex gap-2 whitespace-pre px-2.5", event.seq === 1845 && "bg-[var(--color-code-highlight)]")}>
            <span className="w-10 shrink-0 text-right text-muted-foreground/50">{event.seq}</span>
            <span className={cn("w-[130px] shrink-0", event.tone)}>{event.kind}</span>
            <span className="w-14 shrink-0 text-muted-foreground/70">{event.source}</span>
            <span className="text-muted-foreground/50">{event.time}</span>
          </p>
        ))}
      </div>
      <div className="shrink-0 border-t border-border px-2.5 py-2">
        <p className="text-[10px] uppercase tracking-[0.1em] text-muted-foreground/60">Branch</p>
        <p className="mt-1 font-mono text-[10.5px] text-muted-foreground">
          head <span className="text-foreground">e7·1847</span> · 2 leaves · diverged at <span className="text-foreground">e7·1802</span>
        </p>
      </div>
      <PaneFooter>
        <span>2 148 events</span>
        <span className="ml-auto">copy range as JSON</span>
      </PaneFooter>
    </div>
  );
}

function TasksMock() {
  const rows = [
    { title: "worker · migrate diff rows", state: "running", detail: "4m 12s · turn 6", tone: "bg-success", icon: Activity },
    { title: "bun run dev", state: "running", detail: "1h 04m · shell", tone: "bg-success", icon: TerminalSquare },
    { title: "worker · widen dock tests", state: "queued", detail: "waiting on concurrency (2/2)", tone: "bg-muted-foreground/40", icon: Clock },
    { title: "worker · regenerate protocol", state: "failed", detail: "3 retries · budget exhausted", tone: "bg-destructive", icon: CircleAlert },
    { title: "briefing · morning sweep", state: "done", detail: "finished 08:04", tone: "bg-muted-foreground/25", icon: Check },
  ];
  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto">
        {rows.map(row => {
          const Icon = row.icon;
          return (
            <div
              key={row.title}
              className={cn(
                "group flex items-start gap-2.5 border-b border-border px-2.5 py-2.5 transition-colors hover:bg-accent",
                row.state === "failed" && "bg-destructive/5",
              )}
            >
              <span className={cn("mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full", row.tone, row.state === "running" && "mission-live-accent")} />
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5">
                  <Icon size={11} className="shrink-0 text-muted-foreground" aria-hidden="true" />
                  <b className="min-w-0 truncate text-[11.5px] font-medium text-foreground">{row.title}</b>
                </span>
                <small className={cn("mt-0.5 block text-[10.5px]", row.state === "failed" ? "text-destructive" : "text-muted-foreground")}>
                  {row.detail}
                </small>
              </span>
              {row.state === "failed" && (
                <button type="button" title="Retry" className="hidden h-6 w-6 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground group-hover:inline-flex">
                  <RotateCcw size={11} strokeWidth={1.8} aria-hidden="true" />
                </button>
              )}
              <ChevronRight size={12} className="mt-1 shrink-0 text-muted-foreground/40" aria-hidden="true" />
            </div>
          );
        })}
      </div>
      <PaneFooter>
        <span>2 running · 1 queued</span>
        <span className="ml-auto text-destructive">1 needs you</span>
      </PaneFooter>
    </div>
  );
}

/* ── Preview-only controls ────────────────────────────────────────────────── */

function PreviewControls({ dark, hasRepo, onToggleTheme, onToggleRepo, onReset }: {
  dark: boolean;
  hasRepo: boolean;
  onToggleTheme: () => void;
  onToggleRepo: () => void;
  onReset: () => void;
}) {
  return (
    <div className="u-overlay fixed bottom-10 left-3 z-50 flex items-center gap-0.5 rounded-full px-1.5 py-1 opacity-60 transition-opacity hover:opacity-100">
      <span className="pl-1.5 pr-1 text-[10px] uppercase tracking-[0.1em] text-muted-foreground/70">Mockup</span>
      <button type="button" onClick={onToggleTheme} title="Theme" className="inline-flex h-7 w-7 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
        {dark ? <Sun size={13} aria-hidden="true" /> : <Moon size={13} aria-hidden="true" />}
      </button>
      <button
        type="button"
        onClick={onToggleRepo}
        title="Switch between a workspace session and a chat with no repository"
        className={cn(
          "inline-flex h-7 items-center gap-1.5 rounded-full px-2 text-[11px] transition-colors hover:bg-accent",
          hasRepo ? "text-foreground" : "text-muted-foreground",
        )}
      >
        <MessageSquareText size={12} aria-hidden="true" />
        {hasRepo ? "Repo" : "Direct"}
      </button>
      <button
        type="button"
        onClick={onReset}
        title="Reset the dock  ⌥⌘0 toggle · ⌥⌘1-6 panes · ⌥⌘↩ expand"
        className="inline-flex h-7 w-7 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <RotateCcw size={12} aria-hidden="true" />
      </button>
    </div>
  );
}
