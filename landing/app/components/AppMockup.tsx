import MockEntry, { dotTone } from "./MockEntry";
import type { DockFile, Scene, SidebarProject } from "../content/scenes";

function NavItem({ label }: { label: string }) {
  return (
    <span className="flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-[13px] tracking-[-0.008em] text-foreground/85">
      <span className="size-3.5 shrink-0 rounded-[3px] border border-faint-2" aria-hidden="true" />
      {label}
    </span>
  );
}

function Project({ project }: { project: SidebarProject }) {
  return (
    <div className="mb-2 flex flex-col gap-0.5">
      <div
        className={`flex h-7 w-full items-center gap-2 rounded-md pl-2 pr-1 text-[13px] tracking-[-0.008em] ${
          project.open ? "bg-accent font-medium text-foreground" : ""
        }`}
      >
        <span className="shrink-0 text-faint" aria-hidden="true">
          {project.open ? "⌄" : "›"}
        </span>
        <span className="min-w-0 truncate text-[12px] text-muted-foreground">{project.name}</span>
        <span className="ml-auto shrink-0 text-[11px] tabular-nums text-muted-foreground/60">{project.sessions.length}</span>
      </div>
      {project.sessions.map((session) => (
        <span
          key={session.title + session.time}
          className={`relative flex items-center rounded-[7px] ${
            session.selected ? "bg-selection text-selection-foreground" : ""
          }`}
        >
          <span className="flex h-11 min-w-0 flex-1 items-center gap-2 rounded-[7px] pl-7 pr-2 text-left">
            <span className="flex min-w-0 flex-1 flex-col justify-center gap-0.5">
              <span className="truncate text-[13px] font-medium leading-4 tracking-[-0.008em] text-foreground">
                {session.title}
              </span>
              <span className="flex min-w-0 items-center gap-1.5 truncate text-[11px] leading-[0.875rem] tracking-[-0.004em] text-muted-foreground">
                <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${dotTone[session.tone]}`} aria-hidden="true" />
                <span>{session.status}</span>
                <span aria-hidden="true">·</span>
                <span className="shrink-0 tabular-nums text-faint">{session.time}</span>
              </span>
            </span>
          </span>
        </span>
      ))}
    </div>
  );
}

function FileRow({ file }: { file: DockFile }) {
  return (
    <div className="flex min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1.5">
      <span className="size-1.5 shrink-0 rounded-full bg-faint-2" aria-hidden="true" />
      <span className="flex min-w-0 font-mono text-[12px]">
        <span className="truncate text-muted-foreground">{file.dir}</span>
        <span className="max-w-full shrink-0 truncate text-foreground">{file.name}</span>
      </span>
      <span className="ml-auto flex shrink-0 gap-1.5 pl-2 font-mono text-[11px] tabular-nums">
        <span className="text-success">+{file.added}</span>
        <span className="text-destructive">−{file.removed}</span>
      </span>
    </div>
  );
}

export default function AppMockup({ scene }: { scene: Scene }) {
  const totalFiles = scene.dock.files.length;
  return (
    <div className="w-full overflow-hidden rounded-xl border border-border-card bg-background text-left text-foreground shadow-[0_0_0_1px_#000,0_40px_80px_-30px_rgba(0,0,0,0.9)]">
      <div className="grid h-[620px] grid-cols-[248px_1fr_320px] max-lg:grid-cols-[248px_1fr] max-md:grid-cols-1">
        <aside className="flex min-h-0 flex-col border-r border-border bg-sidebar max-md:hidden">
          <div className="flex h-11 shrink-0 items-center gap-1.5 pl-3.5 pr-1.5">
            <span className="size-3 rounded-full bg-[#ff5f57]" aria-hidden="true" />
            <span className="size-3 rounded-full bg-[#febc2e]" aria-hidden="true" />
            <span className="size-3 rounded-full bg-[#28c840]" aria-hidden="true" />
          </div>
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden px-3 pb-3 pt-1">
            <div className="mb-3 flex shrink-0 items-center gap-1.5">
              <span className="inline-flex h-8 min-w-0 flex-1 items-center gap-2 rounded-[7px] border border-border-card bg-card px-2.5 text-[13px] font-medium text-foreground">
                <span className="min-w-0 flex-1 truncate">New Chat</span>
                <span className="text-[11px] font-normal text-muted-foreground">⌘N</span>
              </span>
              <span className="inline-flex size-8 shrink-0 items-center justify-center rounded-[7px] border border-border text-muted-foreground">
                ⌕
              </span>
            </div>
            <div className="mb-4 shrink-0 space-y-0.5">
              <NavItem label="Marketplace" />
              <NavItem label="Projects" />
              <NavItem label="Memory" />
            </div>
            <div className="min-h-0 flex-1 overflow-hidden">
              <div className="flex h-7 items-center gap-1.5 px-2">
                <span className="text-[12px] font-medium tracking-[-0.004em] text-muted-foreground">Repositories</span>
              </div>
              {scene.projects.map((project) => (
                <Project key={project.name} project={project} />
              ))}
            </div>
            <div className="mt-1 flex shrink-0 items-center gap-0.5 border-t border-sidebar-border pt-1.5">
              <span className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-[12px] text-muted-foreground">
                Settings
              </span>
            </div>
          </div>
        </aside>

        <section className="flex min-h-0 min-w-0 flex-col">
          <div className="flex h-11 shrink-0 select-none items-center gap-2 border-b border-border px-4 sm:px-6">
            <div className="min-w-0 flex-1">
              <h3 className="truncate text-[13px] font-semibold leading-4 text-foreground">{scene.title}</h3>
              <p className="truncate text-[11px] leading-4 text-muted-foreground">{scene.subtitle}</p>
            </div>
            <span className="mx-0.5 h-4 w-px shrink-0 bg-border" aria-hidden="true" />
            <span className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-border bg-card px-2 text-[12px] text-foreground">
              <span className="max-sm:hidden">Review</span>
              <span>{totalFiles} files</span>
              <span className="hidden gap-1.5 pl-1 font-mono text-[11px] tabular-nums lg:inline-flex">
                <span className="text-success">+{scene.dock.added}</span>
                <span className="text-destructive">−{scene.dock.removed}</span>
              </span>
            </span>
          </div>

          <div className="relative min-h-0 flex-1 overflow-hidden">
            <div className="flex flex-col gap-4 px-4 py-4 sm:px-6">
              {scene.entries.map((entry, i) => (
                <MockEntry key={i} entry={entry} />
              ))}
            </div>
            <div
              aria-hidden="true"
              className="pointer-events-none absolute inset-x-0 bottom-0 h-12 bg-linear-to-t from-background to-transparent"
            />
          </div>

          <div className="shrink-0 px-4 pb-4 sm:px-6">
            <div className="flex flex-col rounded-xl border border-border-card bg-card">
              <div className="flex flex-col gap-1 px-3 py-2">
                <span className="px-1 py-0.5 text-[13.5px] leading-relaxed tracking-[-0.006em] text-muted-foreground">
                  Send a follow-up…
                </span>
                <div className="flex min-h-8 items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                    <span className="flex h-8 items-center rounded-md px-2">Claude · Fable 5.1</span>
                    <span className="flex h-8 items-center rounded-md px-2 max-lg:hidden">Work on branch</span>
                  </div>
                  <span className="grid size-7 shrink-0 place-items-center rounded-full bg-primary text-[11px] text-primary-foreground">
                    ↑
                  </span>
                </div>
              </div>
            </div>
          </div>
        </section>

        <aside className="flex min-h-0 flex-col border-l border-border bg-sidebar max-lg:hidden">
          <div className="h-full w-full overflow-hidden px-4 py-4">
            <div className="text-[12px] font-medium text-muted-foreground">CHANGES</div>
            <h3 className="my-1.5 font-heading text-[20px] tracking-[-0.015em] text-foreground">{scene.dock.heading}</h3>
            <p className="mb-2 font-mono text-[11px] text-muted-foreground">{scene.dock.origin}</p>
            <div className="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1.5 font-mono text-[12px]">
              <span className="text-success">+{scene.dock.added}</span>
              <span className="text-destructive">−{scene.dock.removed}</span>
              <span className="flex h-1 min-w-24 flex-1 shrink-0 overflow-hidden rounded-full bg-muted" aria-hidden="true">
                <span
                  className="h-full bg-success"
                  style={{ width: `${Math.round((scene.dock.added / (scene.dock.added + scene.dock.removed)) * 100)}%` }}
                />
                <span className="h-full flex-1 bg-destructive" />
              </span>
            </div>
            <div className="flex flex-col">
              {scene.dock.files.map((file) => (
                <FileRow key={file.dir + file.name} file={file} />
              ))}
            </div>
          </div>
        </aside>
      </div>
    </div>
  );
}
