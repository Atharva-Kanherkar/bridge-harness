import type { Entry, Tone } from "../content/scenes";

export const dotTone: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  info: "bg-info",
  destructive: "bg-destructive",
  faint: "bg-faint",
};

const edgeTone: Record<Tone, string> = {
  success: "border-l-success",
  warning: "border-l-warning",
  info: "border-l-info",
  destructive: "border-l-destructive",
  faint: "border-l-border",
};

const inkTone: Record<Tone, string> = {
  success: "text-success",
  warning: "text-warning",
  info: "text-info",
  destructive: "text-destructive",
  faint: "text-muted-foreground",
};

function Code({ children }: { children: string }) {
  return (
    <code className="mt-1 block max-w-full overflow-x-auto whitespace-pre rounded-md border border-border bg-code px-2.5 py-2 font-mono text-[11.5px] leading-relaxed text-foreground">
      {children}
    </code>
  );
}

export default function MockEntry({ entry }: { entry: Entry }) {
  switch (entry.kind) {
    case "user":
      return (
        <div className="ml-auto w-fit max-w-[85%] rounded-2xl border border-border bg-accent/70 px-4 py-2.5 text-[13.5px] leading-6 tracking-[-0.006em] text-foreground">
          {entry.text}
        </div>
      );

    case "assistant":
      return <p className="w-full min-w-0 text-[13.5px] leading-6 text-body">{entry.text}</p>;

    case "collapsed":
      return (
        <div className="-ml-2 flex min-h-[30px] w-full items-center gap-[9px] rounded-md px-2 py-1 text-[12.5px] text-muted-foreground">
          <span className="size-1 shrink-0 rounded-full bg-faint-2" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate">{entry.label}</span>
          <span className="shrink-0 text-faint" aria-hidden="true">
            ›
          </span>
        </div>
      );

    case "rail":
      return (
        <div className="min-w-0 border-l-2 border-border py-1 pl-3">
          <header className="flex items-center gap-2 text-ui">
            <b className="min-w-0 truncate font-medium text-foreground">{entry.label}</b>
            {entry.status && <small className="ml-auto shrink-0 text-caption text-muted-foreground">{entry.status}</small>}
          </header>
          <p className="mt-1 text-ui text-muted-foreground">{entry.text}</p>
        </div>
      );

    case "notice":
      return (
        <div className={`min-w-0 overflow-hidden rounded-lg border border-l-2 border-border bg-card ${edgeTone[entry.edge]}`}>
          <header className="flex flex-wrap items-baseline gap-x-2 gap-y-1 px-4 pt-3">
            <b className="text-ui font-semibold text-foreground">{entry.title}</b>
            {entry.status && (
              <small className={`text-[11px] tracking-[0.03em] ${inkTone[entry.edge]}`}>{entry.status}</small>
            )}
          </header>
          <p className="mt-1 px-4 text-ui leading-relaxed text-muted-foreground">{entry.text}</p>
          {(entry.caption || entry.code) && (
            <div className="mt-2 px-4">
              {entry.caption && (
                <small className="block text-[11px] uppercase tracking-[0.03em] text-muted-foreground">{entry.caption}</small>
              )}
              {entry.code && <Code>{entry.code}</Code>}
            </div>
          )}
          <div className="flex flex-wrap items-center gap-2 px-4 pb-3 pt-3">
            {entry.actions?.map((action, i) => (
              <span
                key={action}
                className={`inline-flex h-7 items-center rounded-md px-2.5 text-[12px] ${
                  i === entry.actions!.length - 1
                    ? "bg-primary text-primary-foreground"
                    : "border border-border text-muted-foreground"
                }`}
              >
                {action}
              </span>
            ))}
          </div>
        </div>
      );
  }
}
