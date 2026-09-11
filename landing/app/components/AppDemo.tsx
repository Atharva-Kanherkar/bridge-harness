"use client";

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { ArrowUp, Paperclip, PanelRight, Search } from "lucide-react";
import ChangesDock from "./app/ChangesDock";
import MissionGrid from "./app/MissionGrid";
import Sidebar from "./app/Sidebar";
import TranscriptEntry from "./app/Transcript";
import { scenes, type Entry, type Scene } from "../content/appScenes";

const TYPE_MS = 1200;
const STEP_MS = 900;
const HOLD_MS = 3200;
const useIsomorphicLayoutEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;

/** Steps a scene needs before it hands over: the prompt, then one per entry. */
function stepCount(scene: Scene) {
  if (scene.view === "mission") return (scene.tiles?.length ?? 0) + 3;
  return (scene.entries?.length ?? 0) + (scene.prompt ? 1 : 0);
}

function usePlayback(scene: Scene, onDone: () => void) {
  const [typed, setTyped] = useState("");
  const [step, setStep] = useState(stepCount(scene));
  const [playing, setPlaying] = useState(false);
  const done = useRef(onDone);
  useEffect(() => { done.current = onDone; }, [onDone]);

  // Server-rendered markup holds the finished scene, so the demo is complete without
  // JavaScript; playback only arms itself once mounted and motion is welcome.
  useIsomorphicLayoutEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    setTyped("");
    setStep(0);
    setPlaying(true);
  }, [scene.id]);

  useEffect(() => {
    if (!playing) return;
    const timers: number[] = [];
    const at = (delay: number, run: () => void) => timers.push(window.setTimeout(run, delay));
    const prompt = scene.prompt ?? "";
    const total = stepCount(scene);
    let cursor = 320;

    if (prompt) {
      const per = Math.max(12, TYPE_MS / prompt.length);
      for (let i = 1; i <= prompt.length; i += 1) at(cursor + i * per, () => setTyped(prompt.slice(0, i)));
      cursor += prompt.length * per + 420;
      at(cursor, () => { setTyped(""); setStep(1); });
      cursor += STEP_MS;
      for (let i = 2; i <= total; i += 1) { at(cursor, () => setStep(i)); cursor += STEP_MS; }
    } else {
      for (let i = 1; i <= total; i += 1) { at(cursor, () => setStep(i)); cursor += scene.view === "mission" ? 700 : STEP_MS; }
    }

    at(cursor + HOLD_MS, () => done.current());
    return () => timers.forEach(clearTimeout);
  }, [scene, playing]);

  return { typed, step: playing ? step : stepCount(scene), playing };
}

function ChatView({ scene, typed, step }: { scene: Scene; typed: string; step: number }) {
  // The sent prompt is the first thing that lands in the transcript, the way it does in the
  // app, so the pane is never a blank rectangle while the composer is still typing.
  const entries: Entry[] = scene.prompt ? [{ kind: "user", text: scene.prompt }, ...(scene.entries ?? [])] : scene.entries ?? [];
  const shown = entries.slice(0, step);
  const progress = entries.length ? step / entries.length : 1;
  const files = scene.dock ? Math.max(1, Math.ceil(scene.dock.files.length * progress)) : 0;

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
          {shown.map((entry, i) => (
            <div key={i} className="animate-entry-in motion-reduce:animate-none">
              <TranscriptEntry entry={entry} />
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
                <span className="flex h-8 items-center rounded-md px-2">Codex · GPT Luna</span>
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
  const held = useRef(false);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const scene = scenes[active];

  const onDone = useCallback(() => {
    if (held.current) setReplay(value => value + 1);
    else setActive(index => (index + 1) % scenes.length);
  }, []);

  const { typed, step } = usePlayback(scene, onDone);

  function select(index: number) {
    const next = (index + scenes.length) % scenes.length;
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
      <div
        role="tablist"
        aria-label="What Bridge does"
        className="-mx-4 flex gap-2 overflow-x-auto px-4 pb-3 [scrollbar-width:none] sm:mx-0 sm:grid sm:grid-cols-3 sm:overflow-visible sm:px-0 [&::-webkit-scrollbar]:hidden"
      >
        {scenes.map((item, i) => {
          const selected = i === active;
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
              onClick={() => setActive(i)}
              onKeyDown={event => onKeyDown(event, i)}
              className={`group relative flex w-[200px] shrink-0 flex-col overflow-hidden rounded-lg border px-3.5 pb-3.5 pt-3 text-left transition-colors duration-300 sm:w-auto ${
                selected ? "border-border-card bg-card" : "border-transparent hover:bg-card/60"
              }`}
            >
              <span className={`text-[13px] font-medium leading-5 transition-colors ${selected ? "text-foreground" : "text-muted-foreground group-hover:text-foreground"}`}>
                {item.label}
              </span>
              <span className={`mt-1 line-clamp-2 text-[12px] leading-[1.35] transition-colors ${selected ? "text-muted-foreground" : "text-faint"}`}>
                {item.title}
              </span>
              <span aria-hidden="true" className="absolute inset-x-0 bottom-0 h-px bg-border">
                {selected && <span key={`${item.id}:${replay}`} className="block h-full w-full origin-left bg-foreground" />}
              </span>
            </button>
          );
        })}
      </div>

      <p key={scene.id} className="min-h-10 pb-4 text-[13px] leading-5 text-muted-foreground animate-fade-up motion-reduce:animate-none">
        <span className="font-medium text-foreground">{scene.title}. </span>
        {scene.text}
      </p>

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
          {scene.view === "mission" ? <MissionGrid tiles={scene.tiles ?? []} step={step} /> : <ChatView scene={scene} typed={typed} step={step} />}
          {dock && <ChangesDock dock={scene.dock!} progress={Math.max(0.34, step / ((scene.entries?.length ?? 1) + 1))} />}
        </div>
      </div>
    </div>
  );
}
