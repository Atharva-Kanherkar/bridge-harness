import type { ComponentType, ReactNode } from "react";
import { ArrowUp, Check, ChevronDown, CircleDot, FileText, GitBranch, GitPullRequest, MessageSquare, MessagesSquare, Plus, Route, Send, Sparkles, X } from "lucide-react";
import { HarnessMark } from "./harnessMarks";
import { cn } from "@/lib/utils";

type Icon = ComponentType<{ className?: string }>;
type Tone = "plain" | "success" | "warning" | "destructive";

const TONE: Record<Tone, string> = {
  plain: "text-muted-foreground",
  success: "text-success",
  warning: "text-warning",
  destructive: "text-destructive",
};

function Pill({ children, tone = "plain", className }: { children: ReactNode; tone?: Tone; className?: string }) {
  return <span className={`inline-flex h-6 items-center gap-1.5 rounded-md bg-muted px-2 text-caption font-medium ${TONE[tone]} ${className ?? ""}`}>{children}</span>;
}

function Node({ icon: IconComponent, harness, tone = "plain" }: { icon?: Icon; harness?: string; tone?: Tone }) {
  return <span className="relative z-10 flex size-7 shrink-0 items-center justify-center rounded-full bg-card ring-4 ring-background">
    {harness ? <HarnessMark harness={harness} size={14} /> : IconComponent ? <IconComponent className={cn("size-3.5", TONE[tone])} /> : null}
  </span>;
}

function Step({ icon, harness, tone, kind, title, meta, time, children, last }: {
  icon?: Icon; harness?: string; tone?: Tone; kind: string; title: string; meta?: ReactNode; time: string; children?: ReactNode; last?: boolean;
}) {
  return <li className="relative flex gap-3 pb-5">
    {!last && <span aria-hidden className="absolute left-3.5 top-7 h-full w-px bg-border" />}
    <Node icon={icon} harness={harness} tone={tone} />
    <div className="min-w-0 flex-1 pt-0.5">
      <div className="flex items-baseline gap-2">
        <span className="w-24 shrink-0 text-caption uppercase tracking-wide text-faint">{kind}</span>
        <span className="min-w-0 flex-1 truncate text-ui font-medium text-foreground">{title}</span>
        <span className="shrink-0 font-mono text-caption text-faint">{time}</span>
      </div>
      {meta && <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-caption text-muted-foreground">{meta}</div>}
      {children && <div className="mt-2">{children}</div>}
    </div>
  </li>;
}

function Day({ label }: { label: string }) {
  return <li className="relative flex items-center gap-3 pb-3 pt-1">
    <span className="w-7 shrink-0" />
    <span className="text-caption font-medium text-muted-foreground">{label}</span>
  </li>;
}

function Quote({ who, children }: { who: string; children: ReactNode }) {
  return <div className="rounded-lg bg-card px-3 py-2 text-ui leading-relaxed text-body">
    <span className="mr-1.5 font-medium text-foreground">{who}</span>{children}
  </div>;
}

function RailSection({ title, count, children }: { title: string; count?: string; children: ReactNode }) {
  return <section className="rounded-xl bg-card p-4">
    <div className="mb-2.5 flex items-center justify-between">
      <h3 className="text-caption font-medium uppercase tracking-wide text-faint">{title}</h3>
      {count && <span className="font-mono text-caption text-faint">{count}</span>}
    </div>
    {children}
  </section>;
}

function Stages({ done, attention }: { done: number; attention?: number }) {
  return <span className="flex items-center gap-1" aria-hidden>
    {[0, 1, 2, 3, 4].map(index => <span key={index} className={cn("h-1.5 w-5 rounded-full", index === attention ? "bg-warning" : index < done ? "bg-foreground/70" : "bg-muted")} />)}
  </span>;
}

const STAGE_LABELS = ["Issue", "Plan", "Chats", "PR", "Shipped"];

function TrailHeader() {
  return <header className="mb-6">
    <div className="mb-2 flex items-center gap-2 text-caption text-muted-foreground">
      <Route className="size-3.5" /><span>Trails</span><span className="text-faint">/</span><span>bridge-harness</span>
    </div>
    <div className="flex flex-wrap items-start gap-3">
      <h1 className="min-w-0 flex-1 font-display text-title font-semibold tracking-tight text-foreground">Archive a chat from the sidebar</h1>
      <button className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-primary px-3 text-ui font-medium text-primary-foreground"><Plus className="size-3.5" />Continue this trail</button>
    </div>
    <div className="mt-3 flex flex-wrap items-center gap-2">
      <Pill tone="warning"><span className="size-1.5 rounded-full bg-warning" />Needs you</Pill>
      <Pill><CircleDot className="size-3" />#1003</Pill>
      <Pill><GitBranch className="size-3" /><span className="font-mono">feat/archive-chat</span></Pill>
      <Pill><GitPullRequest className="size-3" />#1012 · changes requested</Pill>
      <Pill><MessagesSquare className="size-3" />3 chats</Pill>
      <span className="ml-auto flex items-center gap-2 text-caption text-faint"><Stages done={3} attention={3} />Stage 4 of 5 · PR</span>
    </div>
  </header>;
}

function Timeline() {
  return <ol className="min-w-0 flex-1">
    <Day label="Monday 9 November" />
    <Step icon={CircleDot} kind="Issue" title="#1003 Archive a chat from the sidebar" time="09:12" meta={<><span>opened by you</span><span>·</span><span>GitHub</span></>} />
    <Step harness="claude" kind="Chat" title="Design the archive action" time="09:20" meta={<><span>Claude</span><span>·</span><span>41 min</span><span>·</span><span>wrote the plan note</span></>} />
    <Step icon={FileText} kind="Note" title="Plan: archive action per chat" time="10:01" meta={<><span>edited by you and Claude</span><span>·</span><span>3 decisions</span></>}>
      <ul className="space-y-1 rounded-lg bg-card px-3 py-2 text-ui text-body">
        <li className="flex gap-2"><Check className="mt-1 size-3 shrink-0 text-success" />Archiving keeps the worktree until it is reclaimed.</li>
        <li className="flex gap-2"><Check className="mt-1 size-3 shrink-0 text-success" />Undo for 8 seconds, no confirm dialog.</li>
        <li className="flex gap-2"><Check className="mt-1 size-3 shrink-0 text-success" />Archived chats stay searchable.</li>
      </ul>
    </Step>
    <Day label="Tuesday 10 November" />
    <Step harness="codex" kind="Chat" title="Implement the archive row action" time="11:34" meta={<><span>Codex</span><span>·</span><span>1 h 12 min</span><span>·</span><span>14 files</span><span>·</span><span>started from this trail</span></>} />
    <Step icon={GitPullRequest} kind="Pull request" title="#1012 feat(sidebar): archive a chat from its row" time="12:50" meta={<><span>opened by Codex</span><span>·</span><span className="text-success">+412</span><span className="text-destructive">−38</span><span>·</span><span>closes #1003</span></>} />
    <Step icon={Send} kind="Sent" title="Slack DM to Mika" time="12:58" meta={<><span>drafted by Codex, approved by you</span></>}>
      <Quote who="You">Could you review #1012 today? It adds the archive action we discussed. The undo toast is the part I am least sure about.</Quote>
    </Step>
    <Day label="Wednesday 11 November" />
    <Step icon={MessageSquare} kind="Reply" title="Mika replied in Slack" time="08:40">
      <Quote who="Mika">Looks good. One thing: the undo toast disappears when I switch chats. Left two comments on the PR.</Quote>
    </Step>
    <Step icon={GitPullRequest} tone="warning" kind="Review" title="Changes requested on #1012" time="08:44" last meta={<><span className="text-warning">2 unresolved comments</span><span>·</span><span className="text-destructive">1 failing check</span><span>·</span><span>11 of 12 passing</span></>}>
      <div className="flex items-center gap-2 rounded-lg bg-warning/10 px-3 py-2 text-ui text-foreground">
        <Sparkles className="size-3.5 text-warning" />
        <span className="flex-1">Next step: fix the failing check and answer both comments.</span>
        <button className="inline-flex h-7 items-center gap-1 rounded-md bg-primary px-2.5 text-caption font-medium text-primary-foreground">Start a chat with all of this</button>
      </div>
    </Step>
  </ol>;
}

function Rail() {
  return <aside className="w-72 shrink-0 space-y-3">
    <RailSection title="A new chat starts with" count="~3k tokens">
      <ul className="space-y-2 text-ui text-body">
        <li className="flex gap-2"><FileText className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />The plan note and its 3 decisions</li>
        <li className="flex gap-2"><GitPullRequest className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />PR #1012: diff summary, failing check, 2 comments</li>
        <li className="flex gap-2"><MessagesSquare className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />A short summary of each earlier chat</li>
        <li className="flex gap-2"><MessageSquare className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />What you sent Mika and what Mika replied</li>
      </ul>
    </RailSection>
    <RailSection title="Open" count="3">
      <ul className="space-y-2 text-ui">
        <li className="flex items-center gap-2"><span className="size-1.5 rounded-full bg-destructive" /><span className="flex-1 text-body">Check <span className="font-mono text-caption">vitest</span> failing</span></li>
        <li className="flex items-center gap-2"><span className="size-1.5 rounded-full bg-warning" /><span className="flex-1 text-body">Undo toast lost on chat switch</span></li>
        <li className="flex items-center gap-2"><span className="size-1.5 rounded-full bg-warning" /><span className="flex-1 text-body">Rename <span className="font-mono text-caption">onArchive</span> prop</span></li>
      </ul>
    </RailSection>
    <RailSection title="Agents and people">
      <div className="flex flex-wrap gap-1.5">
        <Pill><HarnessMark harness="claude" size={12} />Claude · 1 chat</Pill>
        <Pill><HarnessMark harness="codex" size={12} />Codex · 2 chats</Pill>
        <Pill>Mika · reviewer</Pill>
      </div>
    </RailSection>
  </aside>;
}

function Page({ children }: { children?: ReactNode }) {
  return <div className="relative h-full overflow-y-auto">
    <div className="mx-auto w-full max-w-page-wide px-8 pb-12 pt-6">
      <TrailHeader />
      <div className="flex gap-8"><Timeline /><Rail /></div>
    </div>
    {children}
  </div>;
}

function Carry({ label, detail, off }: { label: string; detail: string; off?: boolean }) {
  return <li className="flex items-center gap-2.5 py-1.5">
    <span className={cn("flex size-4 items-center justify-center rounded", off ? "bg-muted" : "bg-primary text-primary-foreground")}>{!off && <Check className="size-3" />}</span>
    <span className={`flex-1 text-ui ${off ? "text-muted-foreground" : "text-foreground"}`}>{label}</span>
    <span className="font-mono text-caption text-faint">{detail}</span>
  </li>;
}

function StartDialog() {
  return <div className="absolute inset-0 z-20 flex items-start justify-center bg-scrim pt-24">
    <div className="w-full max-w-xl rounded-2xl bg-popover p-5 shadow-control">
      <div className="mb-3 flex items-center gap-2">
        <Route className="size-4 text-muted-foreground" />
        <h2 className="flex-1 text-ui font-medium text-foreground">New chat on “Archive a chat from the sidebar”</h2>
        <X className="size-4 text-faint" />
      </div>
      <div className="rounded-xl bg-card p-3">
        <p className="min-h-12 text-message text-foreground">Fix the failing vitest check and address both of Mika’s review comments.</p>
        <div className="mt-2 flex items-center gap-2">
          <Pill><HarnessMark harness="claude" size={12} />Claude<ChevronDown className="size-3" /></Pill>
          <Pill><GitBranch className="size-3" /><span className="font-mono">feat/archive-chat</span></Pill>
          <span className="flex-1" />
          <span className="flex size-7 items-center justify-center rounded-full bg-primary text-primary-foreground"><ArrowUp className="size-3.5" /></span>
        </div>
      </div>
      <div className="mt-4">
        <div className="mb-1 flex items-center justify-between">
          <h3 className="text-caption font-medium uppercase tracking-wide text-faint">This chat starts with</h3>
          <span className="font-mono text-caption text-faint">3.1k of 200k tokens</span>
        </div>
        <ul className="divide-y divide-border">
          <Carry label="Plan note and its 3 decisions" detail="0.6k" />
          <Carry label="PR #1012: failing check log and 2 review comments" detail="1.4k" />
          <Carry label="Summary of 2 earlier chats (Claude, Codex)" detail="0.8k" />
          <Carry label="Slack thread with Mika" detail="0.3k" />
          <Carry label="Full transcript of the Codex chat" detail="48k" off />
        </ul>
        <p className="mt-3 text-caption text-muted-foreground">Claude was not in the Codex chat. It gets the summary, so you do not have to explain what happened.</p>
      </div>
    </div>
  </div>;
}

function Row({ title, project, status, tone, done, attention, last, agents }: {
  title: string; project: string; status: string; tone: Tone; done: number; attention?: number; last: string; agents: string[];
}) {
  return <li className="flex items-center gap-4 rounded-xl px-4 py-3 hover:bg-card">
    <div className="min-w-0 flex-1">
      <div className="truncate text-ui font-medium text-foreground">{title}</div>
      <div className="mt-0.5 truncate text-caption text-muted-foreground">{project} · {last}</div>
    </div>
    <span className="flex items-center gap-1">{agents.map(agent => <HarnessMark key={agent} harness={agent} size={13} />)}</span>
    <span className="flex w-40 flex-col gap-1">
      <Stages done={done} attention={attention} />
      <span className="text-caption text-faint">{STAGE_LABELS[Math.min(attention ?? done, 4)]}</span>
    </span>
    <span className={`w-36 text-right text-caption font-medium ${TONE[tone]}`}>{status}</span>
  </li>;
}

function List() {
  return <div className="relative h-full overflow-y-auto">
    <div className="mx-auto w-full max-w-page px-8 pb-12 pt-6">
      <header className="mb-5 flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <h1 className="font-display text-title font-semibold tracking-tight text-foreground">Trails</h1>
          <p className="mt-1 text-ui text-muted-foreground">One page per piece of work: its issue, plan, chats, pull request and messages.</p>
        </div>
        <button className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-primary px-3 text-ui font-medium text-primary-foreground"><Plus className="size-3.5" />New trail</button>
      </header>
      <h2 className="mb-1 px-4 text-caption font-medium uppercase tracking-wide text-faint">Needs you · 2</h2>
      <ul className="mb-5">
        <Row title="Archive a chat from the sidebar" project="bridge-harness" last="Mika requested changes 2 h ago" status="Changes requested" tone="warning" done={3} attention={3} agents={["claude", "codex"]} />
        <Row title="Outgoing: reply watch" project="bridge-harness" last="Plan note waiting for your edits" status="Plan to approve" tone="warning" done={1} attention={1} agents={["codex"]} />
      </ul>
      <h2 className="mb-1 px-4 text-caption font-medium uppercase tracking-wide text-faint">In progress · 3</h2>
      <ul className="mb-5">
        <Row title="Workflows phase 2: schedules" project="bridge-harness" last="Codex is working, 14 min" status="Agent working" tone="plain" done={2} agents={["codex"]} />
        <Row title="Dual subtitles health checks" project="anime-vocab-coach" last="Checks running on #227" status="Checks running" tone="plain" done={3} agents={["claude"]} />
        <Row title="Notes: board view filters" project="bridge-harness" last="You wrote the plan yesterday" status="Not started" tone="plain" done={2} agents={[]} />
      </ul>
      <h2 className="mb-1 px-4 text-caption font-medium uppercase tracking-wide text-faint">Shipped this week · 2</h2>
      <ul>
        <Row title="Mission Control stage and queue" project="bridge-harness" last="Merged Friday" status="Merged" tone="success" done={5} agents={["claude"]} />
        <Row title="Agent drawings that fit the page" project="bridge-harness" last="Merged Friday" status="Merged" tone="success" done={5} agents={["claude", "codex"]} />
      </ul>
    </div>
    <div className="absolute bottom-6 right-6 w-80 rounded-xl bg-popover p-4 shadow-control">
      <div className="flex items-start gap-2.5">
        <Route className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <p className="text-ui font-medium text-foreground">Added to a trail</p>
          <p className="mt-0.5 text-caption leading-relaxed text-muted-foreground">Codex opened PR #1012 from this chat and it closes #1003. Both now sit on “Archive a chat from the sidebar”.</p>
          <div className="mt-2 flex gap-3 text-caption font-medium"><span className="text-foreground">Open trail</span><span className="text-muted-foreground">Undo</span></div>
        </div>
      </div>
    </div>
  </div>;
}

export function TrailMock() {
  const scene = new URLSearchParams(window.location.search).get("trail");
  if (scene === "list") return <List />;
  return <Page>{scene === "start" && <StartDialog />}</Page>;
}
