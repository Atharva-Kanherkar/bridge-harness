import React, { useEffect, useState } from "react";
import {
  AbsoluteFill,
  Audio,
  Img,
  Sequence,
  delayRender,
  continueRender,
  cancelRender,
  interpolate,
  spring,
  staticFile,
  useCurrentFrame,
} from "remotion";
import {
  ArrowRight,
  Check,
  GitBranch,
  ShieldCheck,
  Layers,
  PanelLeft,
  Globe,
  Brain,
  Clock,
  FileCode,
  GitPullRequest,
  Blocks,
  BookOpen,
  ChevronRight,
} from "lucide-react";
import HarnessMark from "../../landing/app/components/app/HarnessMark";
import TranscriptEntry from "../../landing/app/components/app/Transcript";
import { scenes as appScenes } from "../../landing/app/content/appScenes";
import { DURATION, FPS, timeline } from "./scenes";
import "../../node_modules/@fontsource-variable/geist/index.css";
import "../../node_modules/@fontsource-variable/bricolage-grotesque/index.css";
import "../../node_modules/@fontsource-variable/geist-mono/index.css";
import "../../src/index.css";
const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;
function Reveal({
  children,
  delay = 0,
  className = "",
}: {
  children: React.ReactNode;
  delay?: number;
  className?: string;
}) {
  const f = useCurrentFrame();
  const p = spring({
    frame: f - delay,
    fps: FPS,
    config: { damping: 22, stiffness: 95 },
  });
  return (
    <div
      className={className}
      style={{
        opacity: interpolate(f, [delay, delay + 15], [0, 1], clamp),
        transform: `translateY(${(1 - p) * 30}px)`,
      }}
    >
      {children}
    </div>
  );
}
function Badge({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-2 rounded-full border border-border bg-card px-4 py-2 font-mono text-[16px] text-muted-foreground">
      {children}
    </span>
  );
}
function Window({
  children,
  title = "bridge / workspace",
}: {
  children: React.ReactNode;
  title?: string;
}) {
  return (
    <div className="overflow-hidden rounded-2xl border border-border bg-background shadow-2xl">
      <div className="flex h-14 items-center gap-2 border-b border-border bg-sidebar px-6">
        <i className="size-3 rounded-full bg-muted-foreground/40" />
        <i className="size-3 rounded-full bg-muted-foreground/30" />
        <i className="size-3 rounded-full bg-muted-foreground/20" />
        <span className="ml-5 font-mono text-[17px] text-muted-foreground">
          {title}
        </span>
        <PanelLeft size={20} className="ml-auto text-muted-foreground" />
      </div>
      <div className="h-[610px] p-9">{children}</div>
    </div>
  );
}
function Row({
  icon: Icon = FileCode,
  title,
  detail,
  active = false,
}: {
  icon?: typeof FileCode;
  title: string;
  detail: string;
  active?: boolean;
}) {
  return (
    <div
      className={`flex items-center gap-5 rounded-xl border p-5 ${active ? "border-ring/50 bg-accent" : "border-border bg-card"}`}
    >
      <Icon size={26} className="shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="text-[23px] font-medium">{title}</div>
        <div className="mt-1 text-[18px] text-muted-foreground">{detail}</div>
      </div>
      {active ? (
        <Check size={24} className="text-success" />
      ) : (
        <ChevronRight size={23} className="text-muted-foreground" />
      )}
    </div>
  );
}
function Providers() {
  const f = useCurrentFrame();
  const selected = f < 70 ? 0 : f < 125 ? 1 : f < 165 ? 2 : 3;
  return (
    <Window title="bridge / switch agent">
      <div className="mb-7 flex items-center gap-3 text-[21px]">
        <BookOpen size={23} />
        <span>One conversation. More possibilities.</span>
      </div>
      <div className="grid grid-cols-2 gap-4">
        {["Claude Code", "Codex", "Cursor", "OpenCode"].map((name, i) => (
          <Reveal delay={12 + i * 9} key={name}>
            <div
              className={`flex items-center gap-5 rounded-xl border p-7 ${selected === i ? "border-ring bg-accent" : "border-border bg-card"}`}
            >
              <HarnessMark
                harness={["claude", "codex", "cursor", "opencode"][i]}
                size={43}
              />
              <span className="text-[25px]">{name}</span>
              {selected === i && (
                <Check size={23} className="ml-auto text-success" />
              )}
            </div>
          </Reveal>
        ))}
      </div>
      <Reveal
        delay={60}
        className="mt-7 rounded-xl border border-border bg-card p-6"
      >
        <div className="font-mono text-[16px] text-muted-foreground">
          CONVERSATION HISTORY
        </div>
        <p className="mt-3 text-[23px]">
          “Continue from the plan we just reviewed.”
        </p>
        <div className="mt-4 flex gap-2">
          {Array.from({ length: 12 }, (_, i) => (
            <div
              key={i}
              className="h-1.5 flex-1 rounded bg-muted-foreground/40"
            />
          ))}
        </div>
      </Reveal>
    </Window>
  );
}
function Delegation() {
  const f = useCurrentFrame();
  return (
    <Window title="bridge / team activity">
      <div className="mb-8 flex items-center justify-between">
        <div className="text-[30px] font-display">
          Ship the workspace upgrade
        </div>
        <Badge>3 roles</Badge>
      </div>
      <div className="ml-4 space-y-5 border-l border-border pl-8">
        {[
          ["Research", "Map the existing behavior", "fast", "codex"],
          [
            "Implementation",
            "Build in an isolated worktree",
            "standard",
            "claude",
          ],
          [
            "Verification",
            "Review the result independently",
            "strong",
            "codex",
          ],
        ].map(([name, detail, tier, harness], i) => (
          <Reveal delay={20 + i * 27} key={name}>
            <div className="flex gap-5 rounded-xl border border-border bg-card p-5">
              <HarnessMark harness={harness} size={30} />
              <div className="flex-1">
                <div className="flex items-center justify-between text-[23px]">
                  {name}
                  <Badge>{tier}</Badge>
                </div>
                <p className="mt-2 text-[18px] text-muted-foreground">
                  {detail}
                </p>
                <div className="mt-4 h-1 overflow-hidden rounded bg-muted">
                  <div
                    className="h-full bg-success/70"
                    style={{
                      width: `${interpolate(f, [25 + i * 28, 125 + i * 28], [0, 100], clamp)}%`,
                    }}
                  />
                </div>
              </div>
            </div>
          </Reveal>
        ))}
      </div>
    </Window>
  );
}
function Transcript({ id }: { id: string }) {
  const scene = appScenes.find((s) => s.id === id)!;
  const f = useCurrentFrame();
  const entries = scene.entries ?? [];
  const visible =
    id === "verify"
      ? [entries[1], ...(f > 130 ? [entries[2]] : [])]
      : f < 70
        ? entries.slice(0, 2)
        : f < 130
          ? [entries[2]]
          : [entries[3]];
  return (
    <Window
      title={`bridge / ${id === "verify" ? "verification record" : "isolated workspace"}`}
    >
      <div className="mb-6 flex items-center gap-3 text-[26px]">
        <GitBranch size={25} />
        {scene.label}
      </div>
      <div className="origin-top-left scale-[1.5] w-2/3 space-y-4">
        {visible.map((entry, i) => {
          const displayed =
            entry.kind === "checks" && f > 120
              ? {
                  ...entry,
                  title: "Verified",
                  status: "4 of 4 required checks passed",
                  checks: entry.checks.map((c) => ({
                    ...c,
                    state: "passed" as const,
                  })),
                }
              : entry;
          return (
            <Reveal
              delay={12 + i * 15}
              key={`${f < 95 ? "early" : "late"}-${i}`}
            >
              <TranscriptEntry entry={displayed} />
            </Reveal>
          );
        })}
      </div>
    </Window>
  );
}
function History() {
  const f = useCurrentFrame();
  return (
    <Window title="bridge / session history">
      <div className="mb-9 flex gap-3">
        <Badge>Local ledger</Badge>
        <Badge>Fork & rewind</Badge>
        <Badge>Resume</Badge>
      </div>
      <div className="relative ml-6 space-y-7 border-l-2 border-border pl-9">
        {[
          ["09:41", "Plan captured", "Messages, tools and decisions"],
          [
            "10:18",
            "Checkpoint created",
            "Context summarized; original events preserved",
          ],
          [
            "Tomorrow",
            "Conversation resumed",
            "Continue from a known boundary",
          ],
        ].map(([time, title, detail], i) => (
          <Reveal delay={15 + i * 30} key={time}>
            <div className="absolute -left-[49px] top-5 size-5 rounded-full border-4 border-background bg-foreground" />
            <div className="font-mono text-[16px] text-muted-foreground">
              {time}
            </div>
            <div className="mt-2 text-[28px]">{title}</div>
            <div className="mt-1 text-[19px] text-muted-foreground">
              {detail}
            </div>
          </Reveal>
        ))}
      </div>
      <Reveal delay={120} className="mt-7">
        <div className="flex items-center gap-3 text-[20px] text-success">
          <Check size={21} />
          {f > 150 ? "Ready to continue." : "Restoring context…"}
        </div>
      </Reveal>
    </Window>
  );
}
function Memory() {
  return (
    <Window title="bridge / memory">
      <div className="mb-7 flex items-center gap-4">
        <Brain size={30} />
        <h3 className="font-display text-[34px]">Memory</h3>
        <div className="ml-auto flex gap-6 text-[18px]">
          <span className="text-foreground">Knowledge</span>
          <span className="text-muted-foreground">Activity</span>
        </div>
      </div>
      <div className="space-y-4">
        {[
          ["Your preferences", "Keep explanations concise."],
          [
            "Project conventions",
            "Use Tailwind v4 and the existing design tokens.",
          ],
          ["Shared decisions", "Review changes before they land."],
        ].map(([title, detail], i) => (
          <Reveal delay={i * 25 + 15} key={title}>
            <Row icon={BookOpen} title={title} detail={detail} />
          </Reveal>
        ))}
      </div>
      <Reveal delay={105} className="mt-7">
        <Badge>
          <Check size={17} /> Relevant knowledge recalled for this session
        </Badge>
      </Reveal>
    </Window>
  );
}
function Browser() {
  return (
    <Window title="bridge / approved browser tab">
      <div className="rounded-xl border border-border bg-card p-5">
        <div className="flex items-center gap-3 text-[22px]">
          <Globe size={25} /> Project dashboard <Badge>One approved tab</Badge>
        </div>
        <div className="mt-6 grid grid-cols-3 gap-4">
          {["Overview", "Activity", "Reports"].map((t) => (
            <div
              key={t}
              className="rounded-lg border border-border p-4 text-[19px] text-muted-foreground"
            >
              {t}
              <div className="mt-5 h-12 rounded bg-muted" />
            </div>
          ))}
        </div>
      </div>
      <Reveal delay={45} className="mt-5">
        <Row
          icon={ShieldCheck}
          title="Action requires your approval"
          detail="Review the exact browser action before it runs."
          active
        />
      </Reveal>
      <Reveal delay={90} className="mt-6 flex gap-4">
        <Badge>Temporary access</Badge>
        <Badge>Take over</Badge>
        <Badge>Detach</Badge>
      </Reveal>
    </Window>
  );
}
function Extend() {
  return (
    <Window title="bridge / tools & connections">
      <div className="grid grid-cols-2 gap-5">
        {[
          [Blocks, "Plugins", "Add new capabilities"],
          [BookOpen, "Skills", "Reusable workflows"],
          [Layers, "Agent roles", "Guidance for each job"],
          [GitPullRequest, "GitHub", "Issues and pull requests"],
        ].map(([Icon, title, detail], i) => (
          <Reveal delay={i * 22 + 12} key={String(title)}>
            <div className="min-h-52 rounded-xl border border-border bg-card p-7">
              {React.createElement(Icon as typeof Blocks, {
                size: 35,
                className: "text-muted-foreground",
              })}
              <div className="mt-6 text-[28px]">{String(title)}</div>
              <div className="mt-2 text-[19px] text-muted-foreground">
                {String(detail)}
              </div>
            </div>
          </Reveal>
        ))}
      </div>
    </Window>
  );
}
function Prompts() {
  return (
    <Window title="bridge / prompt studio">
      <div className="mb-7 flex items-center justify-between">
        <h3 className="text-[32px] font-display">
          Guide how your agents work.
        </h3>
        <Badge>Revision history</Badge>
      </div>
      <div className="rounded-xl border border-border bg-code p-7 font-mono text-[20px] leading-9">
        <div className="text-muted-foreground">
          Implementation role / proposed update
        </div>
        <div className="mt-5 text-destructive">
          − Add a new abstraction if needed.
        </div>
        <Reveal delay={30}>
          <div className="mt-3 text-success">
            + Check the existing API before adding another abstraction.
          </div>
        </Reveal>
      </div>
      <Reveal delay={75} className="mt-6">
        <Row
          icon={ShieldCheck}
          title="You approve the exact change"
          detail="Inspect the proposal, rationale and revision history."
        />
      </Reveal>
    </Window>
  );
}
function Automations() {
  return (
    <Window title="bridge / automations">
      <div className="mb-7 flex items-center justify-between">
        <h3 className="text-[32px] font-display">A little less busywork.</h3>
        <Clock size={30} />
      </div>
      <div className="space-y-4">
        {[
          ["Issue triage", "Every weekday · 09:00"],
          ["Dependency review", "Every Monday · 10:00"],
          ["Workspace maintenance", "Every Friday · 16:00"],
        ].map(([title, detail], i) => (
          <Reveal delay={i * 25 + 10} key={title}>
            <Row icon={Clock} title={title} detail={detail} active />
          </Reveal>
        ))}
      </div>
      <Reveal delay={110} className="mt-6">
        <Badge>
          <GitBranch size={18} /> Each run gets a worktree and reports back
        </Badge>
      </Reveal>
    </Window>
  );
}
function Screenshot({ name }: { name: string }) {
  const f = useCurrentFrame();
  return (
    <div className="relative overflow-hidden rounded-2xl border border-border bg-background shadow-2xl">
      <Img
        src={staticFile(`media/${name}`)}
        className="w-full"
        style={{
          transform: `scale(${interpolate(f, [0, 210], [1, 1.045], clamp)})`,
        }}
      />
    </div>
  );
}
function Visual({ id, image }: { id: string; image?: string }) {
  if (image) return <Screenshot name={image} />;
  switch (id) {
    case "providers":
      return <Providers />;
    case "delegate":
      return <Delegation />;
    case "worktrees":
    case "verify":
      return <Transcript id={id} />;
    case "history":
      return <History />;
    case "memory":
      return <Memory />;
    case "browser":
      return <Browser />;
    case "extend":
      return <Extend />;
    case "prompts":
      return <Prompts />;
    case "automations":
      return <Automations />;
    default:
      return null;
  }
}
function Brand() {
  return (
    <div className="flex items-center gap-4">
      <div className="flex h-9 w-10 items-end justify-between">
        <span className="h-7 w-2 rounded-t bg-foreground" />
        <span className="h-9 w-2 rounded-t bg-foreground" />
        <span className="h-7 w-2 rounded-t bg-foreground" />
      </div>
      <span className="font-display text-[32px] font-semibold tracking-tight">
        Bridge
      </span>
    </div>
  );
}
function Scene({
  scene,
  index,
}: {
  scene: (typeof timeline)[number];
  index: number;
}) {
  const f = useCurrentFrame();
  const opacity = interpolate(
    f,
    [0, 12, scene.duration - 12, scene.duration],
    [0, 1, 1, 0],
    clamp,
  );
  const bookend = scene.id === "intro" || scene.id === "outro";
  return (
    <AbsoluteFill className="bg-background text-foreground" style={{ opacity }}>
      <div className="absolute inset-x-20 top-14 flex items-center justify-between">
        <Brand />
        <span className="font-mono text-[16px] tracking-widest text-muted-foreground">
          THE CONTROL ROOM FOR CODING AGENTS
        </span>
      </div>
      {bookend ? (
        <div className="absolute inset-0 flex flex-col items-center justify-center px-20 text-center">
          <Reveal delay={6}>
            <p className="font-mono text-[19px] tracking-[0.3em] text-muted-foreground">
              {scene.eyebrow}
            </p>
          </Reveal>
          <Reveal delay={16}>
            <h1 className="mt-7 whitespace-pre-line font-display text-[112px] font-semibold leading-[1.04] tracking-[-0.055em]">
              {scene.title}
            </h1>
          </Reveal>
          <Reveal delay={32}>
            <p className="mt-9 text-[30px] text-muted-foreground">
              {scene.body}
            </p>
          </Reveal>
          <Reveal delay={48} className="mt-12 flex items-center gap-8">
            {scene.id === "intro" ? (
              ["claude", "codex", "cursor", "opencode"].map((h) => (
                <div
                  key={h}
                  className="rounded-2xl border border-border bg-card p-5"
                >
                  <HarnessMark harness={h} size={44} />
                </div>
              ))
            ) : (
              <div className="flex items-center gap-4 rounded-full bg-primary px-9 py-5 text-[26px] text-primary-foreground">
                Get Bridge <ArrowRight size={28} />
              </div>
            )}
          </Reveal>
          {scene.id === "outro" && (
            <Reveal delay={65}>
              <p className="mt-8 font-mono text-[20px] text-muted-foreground">
                github.com/Atharva-Kanherkar/bridge-harness
              </p>
            </Reveal>
          )}
        </div>
      ) : (
        <div className="absolute inset-x-20 top-[215px] flex items-center gap-16">
          <div className="w-[480px] shrink-0">
            <Reveal>
              <p className="font-mono text-[17px] tracking-[0.12em] text-muted-foreground">
                {scene.eyebrow}
              </p>
            </Reveal>
            <Reveal delay={10}>
              <h2 className="mt-7 whitespace-pre-line font-display text-[66px] font-semibold leading-[1.04] tracking-[-0.045em]">
                {scene.title}
              </h2>
            </Reveal>
            <Reveal delay={23}>
              <p className="mt-8 max-w-[450px] text-[25px] leading-[1.5] text-muted-foreground">
                {scene.body}
              </p>
            </Reveal>
            {"note" in scene && (
              <Reveal delay={60}>
                <p className="mt-8 border-l-2 border-border pl-5 text-[19px] leading-relaxed text-muted-foreground">
                  {scene.note}
                </p>
              </Reveal>
            )}
          </div>
          <Reveal delay={12} className="relative w-[1216px]">
            <Visual
              id={scene.id}
              image={"image" in scene ? scene.image : undefined}
            />
          </Reveal>
        </div>
      )}
      <div className="absolute inset-x-20 bottom-12 flex items-center justify-between text-[15px] font-mono text-muted-foreground">
        <span>
          {bookend
            ? "BUILT AROUND YOUR WORK"
            : "PRODUCT WALKTHROUGH · ILLUSTRATIVE WORKFLOWS"}
        </span>
        <div className="flex items-center gap-2">
          {timeline.map((s, i) => (
            <div
              key={s.id}
              className={`h-1 w-7 rounded-full ${i <= index ? "bg-foreground" : "bg-border"}`}
            />
          ))}
        </div>
        <span>
          {String(index + 1).padStart(2, "0")} / {timeline.length}
        </span>
      </div>
    </AbsoluteFill>
  );
}
export function Film() {
  const [fontHandle] = useState(() => delayRender("Load Bridge typography"));
  useEffect(() => {
    Promise.all([
      document.fonts.load('600 64px "Bricolage Grotesque Variable"'),
      document.fonts.load('400 24px "Geist Variable"'),
      document.fonts.load('400 20px "Geist Mono Variable"'),
    ])
      .then(() => document.fonts.ready)
      .then(
        () =>
          new Promise<void>((resolve) => {
            requestAnimationFrame(() =>
              requestAnimationFrame(() => setTimeout(resolve, 150)),
            );
          }),
      )
      .then(() => continueRender(fontHandle))
      .catch(cancelRender);
  }, [fontHandle]);
  return (
    <AbsoluteFill className="dark font-sans [&_*]:animate-none [&_*]:transition-none">
      <AbsoluteFill className="bg-background" />
      <Audio
        src={staticFile("score.wav")}
        volume={(f) =>
          interpolate(
            f,
            [0, 30, DURATION - 60, DURATION],
            [0, 0.72, 0.72, 0],
            clamp,
          )
        }
      />
      {timeline.map((scene, index) => (
        <Sequence
          key={scene.id}
          from={scene.from}
          durationInFrames={scene.duration}
        >
          <Scene scene={scene} index={index} />
        </Sequence>
      ))}
    </AbsoluteFill>
  );
}
