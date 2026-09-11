import { Check, ChevronRight, Code2, Columns2 } from "lucide-react";
import type { Dock } from "../../content/appScenes";

const riskTone: Record<string, string> = {
  High: "border-destructive/40 text-destructive",
  Medium: "border-warning/40 text-warning",
  Low: "border-border-card text-muted-foreground",
};

/** The Changes pane, counting up as the worker's diff lands. */
export default function ChangesDock({ dock, progress }: { dock: Dock; progress: number }) {
  const files = dock.files.slice(0, Math.max(1, Math.ceil(dock.files.length * progress)));
  const added = Math.round(dock.added * progress);
  const removed = Math.round(dock.removed * progress);
  const split = added + removed > 0 ? Math.round((added / (added + removed)) * 100) : 0;

  return (
    <aside className="flex min-h-0 flex-col border-l border-border bg-sidebar max-lg:hidden">
      <div className="h-full w-full overflow-hidden px-4 py-4">
        <div className="text-[12px] font-medium text-muted-foreground">CHANGES</div>
        <h3 className="my-1.5 font-heading text-[20px] tracking-[-0.015em] text-foreground">
          {files.length} file{files.length === 1 ? "" : "s"} changed
        </h3>
        <p className="mb-2 truncate font-mono text-[11px] text-muted-foreground">{dock.origin}</p>

        <div className="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1.5 font-mono text-[12px]">
          <span className="tabular-nums text-success">+{added}</span>
          <span className="tabular-nums text-destructive">−{removed}</span>
          <span className="flex h-1 min-w-16 flex-1 shrink-0 overflow-hidden rounded-full bg-muted" aria-hidden="true">
            <span className="h-full bg-success transition-[width] duration-700" style={{ width: `${split}%` }} />
            <span className="h-full flex-1 bg-destructive" />
          </span>
          <span className="shrink-0 text-[11px] text-faint">0/{dock.files.length} viewed</span>
        </div>

        <div className="flex flex-col gap-1.5">
          {files.map(file => (
            <div key={file.dir + file.name} className="animate-entry-in rounded-md border border-border-card bg-card px-2 py-1.5 motion-reduce:animate-none">
              <div className="flex min-w-0 items-center gap-1.5">
                <ChevronRight size={12} className="shrink-0 text-faint" aria-hidden="true" />
                <span className="flex min-w-0 font-mono text-[11.5px]">
                  <span className="truncate text-muted-foreground">{file.dir}</span>
                  <span className="shrink-0 text-foreground">{file.name}</span>
                </span>
              </div>
              <div className="mt-1.5 flex items-center gap-1.5 pl-[18px]">
                <Columns2 size={11} className="shrink-0 text-faint-2" aria-hidden="true" />
                <Code2 size={11} className="shrink-0 text-faint-2" aria-hidden="true" />
                {file.lang && <span className="rounded border border-border-card px-1 py-px font-mono text-[10px] text-muted-foreground">{file.lang}</span>}
                {file.risk && <span className={`rounded border px-1 py-px font-mono text-[10px] ${riskTone[file.risk]}`}>{file.risk}</span>}
                <span className="ml-auto flex shrink-0 items-center gap-1.5 font-mono text-[10.5px] tabular-nums">
                  <span className="text-success">+{file.added}</span>
                  <span className="text-destructive">−{file.removed}</span>
                  <span className="flex h-1 w-10 overflow-hidden rounded-full bg-muted" aria-hidden="true">
                    <span className="h-full bg-success" style={{ width: `${Math.round((file.added / (file.added + file.removed)) * 100)}%` }} />
                    <span className="h-full flex-1 bg-destructive" />
                  </span>
                  <Check size={11} className="text-faint-2" aria-hidden="true" />
                </span>
              </div>
            </div>
          ))}
        </div>

        <p className="mt-3 text-[11px] text-faint">1 low-signal file hidden — show</p>
      </div>
    </aside>
  );
}
