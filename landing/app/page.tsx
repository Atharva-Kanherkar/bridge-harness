import AppMockup from "./components/AppMockup";

const tabs = ["Orchestrate a fleet", "Policy owns the gates", "Isolated worktrees", "Verify across harnesses", "Track usage"];

export default function Home() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <header className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
        <span className="text-sm font-semibold tracking-tight">Bridge</span>
        <nav className="flex items-center gap-6 text-[13px] text-muted-foreground">
          <a href="#" className="hover:text-foreground">Docs</a>
          <a href="#" className="hover:text-foreground">Changelog</a>
          <a href="#" className="hover:text-foreground">Blog</a>
          <a href="#" className="rounded-md bg-foreground px-3 py-1.5 text-background hover:bg-foreground/90">Download</a>
        </nav>
      </header>

      <main className="mx-auto flex max-w-6xl flex-col items-center px-6 pb-24 pt-16 text-center">
        <span className="text-[13px] text-muted-foreground">Open source · macOS</span>
        <h1 className="mt-5 max-w-4xl text-5xl font-semibold leading-[1.05] tracking-tight sm:text-7xl">
          Delegate the coding.<br />Keep the judgment.
        </h1>
        <p className="mt-6 max-w-2xl text-lg leading-8 text-muted-foreground">
          Bridge runs Codex, Claude Code, and OpenCode as one team. An orchestrator plans and routes, a policy
          engine owns every safety gate, and each worker lands in its own worktree with verifiable evidence.
        </p>
        <div className="mt-8 flex flex-col gap-3 sm:flex-row">
          <a href="#" className="rounded-md bg-foreground px-5 py-3 text-sm font-medium text-background hover:bg-foreground/90">Download for Mac</a>
          <a href="#" className="rounded-md border border-border-card px-5 py-3 text-sm font-medium hover:bg-muted">View on GitHub</a>
        </div>

        <div className="mt-20 inline-flex rounded-lg border border-border bg-card p-1 text-[13px]">
          {tabs.map((t, i) => (
            <span key={t} className={`rounded-md px-3 py-1.5 ${i === 0 ? "bg-muted text-foreground" : "text-muted-foreground"}`}>{t}</span>
          ))}
        </div>

        <div className="mt-6 w-full">
          <AppMockup />
        </div>
      </main>
    </div>
  );
}
