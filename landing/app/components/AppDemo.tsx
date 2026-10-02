"use client";

import { useCallback, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent } from "react";
import { ArrowLeftRight, ArrowUp, BadgeCheck, GitBranch, LayoutGrid, Paperclip, PanelRight, Search, ShieldCheck } from "lucide-react";
import ChangesDock from "./app/ChangesDock";
import HarnessMark from "./app/HarnessMark";
import MissionGrid, { missionTicks } from "./app/MissionGrid";
import Sidebar from "./app/Sidebar";
import TranscriptEntry, { entryTicks } from "./app/Transcript";
import { scenes, type Scene } from "../content/appScenes";

/** One tick of the player. Every scene is a pure function of how many ticks have passed. */
const TICK_MS = 60;
/** Ticks the finished scene holds before the strip moves on. */
const HOLD = 55;
/** Prompt characters typed per tick. */
const TYPE_RATE = 2;
/** A tick count past any scene's end, so a paused or server-rendered scene draws finished. */
const FINISHED = Number.MAX_SAFE_INTEGER;
const useIsomorphicLayoutEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;
const REDUCED = "(prefers-reduced-motion: reduce)";

function useReducedMotion() {
  return useSyncExternalStore(
    notify => {
      const query = window.matchMedia(REDUCED);
      query.addEventListener("change", notify);
      return () => query.removeEventListener("change", notify);
    },
    () => window.matchMedia(REDUCED).matches,
    () => false,
  );
}

/** One glyph per scene, so the strip reads as a place rather than a row of words. */
const sceneIcon: Record<string, typeof GitBranch> = {
  worktrees: GitBranch,
  mission: LayoutGrid,
  policy: ShieldCheck,
  verify: BadgeCheck,
  switch: ArrowLeftRight,
};

type Timeline = { sent: number; starts: number[]; ticks: number[]; end: number; total: number };

/** When the prompt sends and when each entry starts; entries play one after another. */
function timeline(scene: Scene): Timeline {
  if (scene.view === "mission") {
    const end = missionTicks(scene.tiles ?? []);
    return { sent: 0, starts: [], ticks: [], end, total: end + HOLD };
  }
  const entries = scene.entries ?? [];
  const sent = scene.prompt ? Math.ceil(scene.prompt.length / TYPE_RATE) + 8 : 0;
  let at = sent + (scene.prompt ? 6 : 4);
  const starts: number[] = [];
  const ticks: number[] = [];
  for (const entry of entries) {
    starts.push(at);
    ticks.push(entryTicks(entry));
    at += entryTicks(entry) + 5;
  }
  return { sent, starts, ticks, end: at, total: at + HOLD };
}

function usePlayback(scene: Scene, replay: number, paused: boolean, onDone: () => void) {
  const [t, setT] = useState(FINISHED);
  const [playing, setPlaying] = useState(false);
  const plan = timeline(scene);
  const done = useRef(onDone);
  useEffect(() => { done.current = onDone; }, [onDone]);

  // Server-rendered markup holds the finished scene, so the demo is complete without
  // JavaScript; playback only arms itself once mounted and the visitor has not paused it.
  // While paused the finished scene is shown, so a selected tab is readable at once.
  useIsomorphicLayoutEffect(() => {
    if (paused) {
      setPlaying(false);
      return;
    }
    setT(0);
    setPlaying(true);
  }, [scene.id, replay, paused]);

  useEffect(() => {
    if (!playing) return;
    const id = window.setInterval(() => setT(value => value + 1), TICK_MS);
    return () => window.clearInterval(id);
  }, [playing, scene.id, replay]);

  useEffect(() => {
    if (playing && t === plan.total) done.current();
  }, [playing, t, plan.total]);

  const now = playing ? t : FINISHED;
  return { t: now, plan, progress: playing ? Math.min(1, t / plan.total) : 0 };
}

function ChatView({ scene, t, plan }: { scene: Scene; t: number; plan: Timeline }) {
  // The sent prompt is the first thing that lands in the transcript, the way it does in the
  // app, so the pane is never a blank rectangle while the composer is still typing.
  const entries = scene.entries ?? [];
  const typed = scene.prompt && t < plan.sent ? scene.prompt.slice(0, Math.min(scene.prompt.length, t * TYPE_RATE)) : "";
  const first = plan.starts[0] ?? 0;
  // The same floor the dock uses, so the toolbar count and the dock always agree.
  const progress = Math.max(0.34, Math.min(1, Math.max(0, (t - first) / Math.max(1, plan.end - first))));
  const files = scene.dock ? Math.max(1, Math.ceil(scene.dock.files.length * progress)) : 0;
  const played = entries.map((entry, i) => ({ entry, p: Math.min(1, (t - plan.starts[i]) / plan.ticks[i]), start: plan.starts[i] })).filter(item => t >= item.start);
  const switched = [...played].reverse().find(item => item.entry.kind === "switch" && item.p >= 1)?.entry;
  const model = switched?.kind === "switch" ? { harness: switched.to, label: switched.model } : scene.model ?? { harness: "codex" as const, label: "Codex · GPT Luna" };

  return (
    <section className="flex min-h-0 min-w-0 flex-col">
      <div className="flex h-11 shrink-0 select-none items-center gap-2 border-b border-border px-4 sm:px-6">
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-[13px] font-semibold leading-4 text-foreground">{scene.toolbar?.title}</h3>
          <p className="truncate text-[11px] leading-4 text-muted-foreground">{scene.toolbar?.subtitle}</p>
        </div>
        <Search size={14} className="shrink-0 text-muted-foreground max-sm:hidden" aria-hidden="true" />
        <PanelRight size={14} className="shrink-0 text-muted-foreground max-sm:hidden" aria-hidden="true" />
        <span className="mx-0.5 h-4 w-px shrink-0 bg-border" aria-hidden="true" />
        <span className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-border bg-card px-2 text-[12px] text-foreground">
          <span className="max-sm:hidden">Review</span>
          <span className="tabular-nums">{files} {files === 1 ? "file" : "files"}</span>
          {scene.dock && (
            <span className="hidden gap-1.5 pl-1 font-mono text-[11px] tabular-nums lg:inline-flex">
              <span className="text-success">+{Math.round(scene.dock.added * progress)}</span>
              <span className="text-destructive">−{Math.round(scene.dock.removed * progress)}</span>
            </span>
          )}
        </span>
      </div>

      <div className="relative flex min-h-0 flex-1 flex-col justify-end overflow-hidden">
        <div className="flex flex-col gap-4 px-4 py-4 sm:px-6">
          {scene.history?.map((entry, i) => (
            <div key={`history-${i}`}>
              <TranscriptEntry entry={entry} />
            </div>
          ))}
          {scene.prompt && t >= plan.sent && (
            <div className="animate-entry-in motion-reduce:animate-none">
              <TranscriptEntry entry={{ kind: "user", text: scene.prompt }} />
            </div>
          )}
          {played.map(({ entry, p }, i) => (
            <div key={i} className="animate-entry-in motion-reduce:animate-none">
              <TranscriptEntry entry={entry} p={p} />
            </div>
          ))}
        </div>
        <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 top-0 h-10 bg-linear-to-b from-background to-transparent" />
      </div>

      <div className="shrink-0 px-4 pb-4 sm:px-6">
        <div className="flex flex-col rounded-xl border border-border-card bg-card">
          <div className="flex flex-col gap-1 px-3 py-2">
            <span className="px-1 py-0.5 text-[13.5px] leading-relaxed tracking-[-0.006em]">
              {typed ? (
                <>
                  <span className="text-foreground">{typed}</span>
                  <span className="ml-px inline-block h-[1.1em] w-px translate-y-[0.18em] bg-foreground animate-caret motion-reduce:animate-none" />
                </>
              ) : (
                <span className="text-muted-foreground">Send a follow-up…</span>
              )}
            </span>
            <div className="flex min-h-8 items-center justify-between gap-2">
              <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                <span key={model.label} className="flex h-8 items-center gap-1.5 rounded-md px-2 animate-entry-in motion-reduce:animate-none">
                  <HarnessMark harness={model.harness} size={12} />
                  {model.label}
                </span>
                <span className="flex h-8 items-center rounded-md px-2 max-lg:hidden">User approval</span>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <Paperclip size={13} className="text-muted-foreground" aria-hidden="true" />
                <span className="grid size-7 place-items-center rounded-full bg-primary text-primary-foreground">
                  <ArrowUp size={13} aria-hidden="true" />
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

export default function AppDemo() {
  const [active, setActive] = useState(0);
  const [replay, setReplay] = useState(0);
  // Auto-advance stops the moment a visitor takes over by clicking or focusing a tab, and stays
  // stopped: the strip then only moves when they pick another tab (WCAG 2.2.2: moving content
  // that runs longer than five seconds needs a stop the user controls, and a hover hold is not
  // one). Reduced-motion visitors start paused; server markup is the finished scene either way.
  const [choice, setChoice] = useState<boolean | null>(null);
  const reducedMotion = useReducedMotion();
  const paused = choice ?? reducedMotion;
  const setPaused = setChoice;
  const held = useRef(false);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const listRef = useRef<HTMLDivElement>(null);
  const [slider, setSlider] = useState<{ left: number; top: number; width: number; height: number } | null>(null);
  const scene = scenes[active];

  // The indicator is measured rather than fractioned, so it fits each label instead of
  // forcing four columns to the width of the longest one. Measuring top too means the
  // indicator still tracks correctly when the tab list wraps onto a second row on mobile.
  useIsomorphicLayoutEffect(() => {
    const measure = () => {
      const tab = tabRefs.current[active];
      const list = listRef.current;
      if (!tab || !list) return;
      // Layout offsets, not bounding rects: the tabs mount mid entrance animation, and a
      // rect would measure them where the transform has them, not where they land.
      setSlider({ left: tab.offsetLeft, top: tab.offsetTop, width: tab.offsetWidth, height: tab.offsetHeight });
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [active]);

  const onDone = useCallback(() => {
    if (held.current) setReplay(value => value + 1);
    else setActive(index => (index + 1) % scenes.length);
  }, []);

  const { t, plan, progress } = usePlayback(scene, replay, paused, onDone);

  function select(index: number) {
    const next = (index + scenes.length) % scenes.length;
    setPaused(true);
    setActive(next);
    tabRefs.current[next]?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const keys: Record<string, number> = { ArrowRight: index + 1, ArrowLeft: index - 1, Home: 0, End: scenes.length - 1 };
    if (!(event.key in keys)) return;
    event.preventDefault();
    select(keys[event.key]);
  }

  const dock = scene.view === "chat" && scene.dock;

  return (
    <div
      className="flex flex-col"
      onPointerEnter={() => { held.current = true; }}
      onPointerLeave={() => { held.current = false; }}
      onFocusCapture={() => { held.current = true; }}
      onBlurCapture={() => { held.current = false; }}
    >
      <div className="-mx-4 mb-5 flex justify-center px-4 sm:overflow-x-auto sm:[scrollbar-width:none] sm:[&::-webkit-scrollbar]:hidden">
        <div
          ref={listRef}
          role="tablist"
          aria-label="What Bridge does"
          className="relative flex w-full max-w-full flex-wrap items-center justify-center gap-1 rounded-2xl border border-border-card bg-card/50 p-1.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.06)] backdrop-blur-xl motion-safe:animate-[entry-in_600ms_ease-out] sm:h-10 sm:w-max sm:shrink-0 sm:flex-nowrap sm:justify-start sm:gap-0 sm:rounded-full sm:p-1"
        >
          {/* The slider the labels ride on. */}
          <span
            aria-hidden="true"
            className="absolute left-0 top-0 rounded-full bg-foreground/12 ring-1 ring-inset ring-foreground/20 transition-[transform,width,height] duration-[400ms] ease-[cubic-bezier(0.68,-0.55,0.265,1.55)] motion-reduce:transition-none"
            style={slider ? { width: slider.width, height: slider.height, transform: `translate(${slider.left}px, ${slider.top}px)` } : { opacity: 0 }}
          >
            {/* How far the current scene has played, so the strip reads as a timeline. */}
            <span className="absolute inset-0 overflow-hidden rounded-full">
              <span className="absolute inset-y-0 left-0 bg-foreground/12" style={{ width: `${progress * 100}%` }} />
            </span>
          </span>

          {scenes.map((item, i) => {
            const selected = i === active;
            const Icon = sceneIcon[item.id];
            return (
              <button
                key={item.id}
                ref={el => { tabRefs.current[i] = el; }}
                type="button"
                role="tab"
                id={`scene-tab-${item.id}`}
                aria-selected={selected}
                aria-controls="scene-panel"
                tabIndex={selected ? 0 : -1}
                onClick={() => select(i)}
                onFocus={() => setPaused(true)}
                onKeyDown={event => onKeyDown(event, i)}
                style={{ animationDelay: `${100 + i * 90}ms` }}
                className={`group relative z-10 flex h-9 items-center gap-1.5 whitespace-nowrap rounded-full px-4 text-[12.5px] font-semibold transition-colors duration-300 motion-safe:animate-[rise_500ms_ease-out_backwards] sm:h-full ${
                  selected ? "text-foreground" : "text-muted-foreground hover:text-foreground"
                }`}
              >
                <span
                  aria-hidden="true"
                  className={`absolute inset-0 rounded-full bg-foreground/10 opacity-0 transition-opacity duration-300 ${selected ? "" : "group-hover:opacity-100"}`}
                />
                {Icon && <Icon size={13} className="relative" aria-hidden="true" />}
                <span className="relative">{item.label}</span>
              </button>
            );
          })}
        </div>
      </div>

      <div
        id="scene-panel"
        role="tabpanel"
        aria-labelledby={`scene-tab-${scene.id}`}
        className="overflow-hidden rounded-xl border border-border-card bg-background shadow-[0_0_0_1px_#000,0_30px_90px_-30px_rgba(0,0,0,0.9)]"
      >
        <div
          className={`grid h-[440px] sm:h-[520px] lg:h-[620px] ${
            dock ? "grid-cols-[248px_minmax(0,1fr)_320px] max-lg:grid-cols-[248px_minmax(0,1fr)]" : "grid-cols-[248px_minmax(0,1fr)]"
          } max-md:grid-cols-1`}
        >
          <Sidebar activeNav={scene.view === "mission" ? "mission" : "chats"} />
          {scene.view === "mission" ? <MissionGrid tiles={scene.tiles ?? []} t={t} /> : <ChatView scene={scene} t={t} plan={plan} />}
          {dock && <ChangesDock dock={scene.dock!} progress={Math.max(0.34, Math.min(1, Math.max(0, (t - (plan.starts[0] ?? 0)) / Math.max(1, plan.end - (plan.starts[0] ?? 0)))))} />}
        </div>
      </div>
    </div>
  );
}
