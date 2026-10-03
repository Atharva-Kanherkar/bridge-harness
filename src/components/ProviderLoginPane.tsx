import { useEffect, useRef, useState } from "react";
import { ChevronDown, ExternalLink, LoaderCircle, ShieldCheck, Terminal } from "lucide-react";
import { bridgeApi } from "../api";
import type { UsageProvider } from "../usage";
import { plainProviderLoginOutput, providerLoginCode, providerLoginUrl } from "../providerLoginPresentation";

// `bare` drops the card and header for hosts that supply their own (the
// sign-in dialog). `onExited` lets that host judge the result instead of
// closing the moment the vendor process ends.
export function ProviderLoginPane({ provider, label, onClose, onExited, bare = false }: {
  provider: UsageProvider | "github" | "grok";
  label: string;
  onClose: () => void;
  onExited?: (output: string) => void;
  bare?: boolean;
}) {
  const [output, setOutput] = useState("");
  const outputTextRef = useRef("");
  const [entry, setEntry] = useState("");
  const [error, setError] = useState<string | null>(null);
  const outputRef = useRef<HTMLPreElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const exitedRef = useRef(onExited);
  exitedRef.current = onExited;
  // Listen before launch: both subscriptions must be registered before the
  // vendor process starts, or its first output (OAuth URL, initial prompt)
  // can be lost permanently.
  useEffect(() => {
    let alive = true;
    let unlistenOutput: (() => void) | undefined;
    let unlistenExit: (() => void) | undefined;
    let launched = false;
    let exited = false;
    void (async () => {
      unlistenOutput = await bridgeApi.onTerminal(chunk => {
        if (!alive || chunk.sessionId !== "provider-login" || chunk.terminalId !== provider) return;
        outputTextRef.current = (outputTextRef.current + chunk.data).slice(-8000);
        setOutput(outputTextRef.current);
      });
      unlistenExit = await bridgeApi.onTerminalExited(exit => {
        if (!alive || exit.sessionId !== "provider-login" || exit.terminalId !== provider) return;
        exited = true;
        if (exitedRef.current) exitedRef.current(plainProviderLoginOutput(outputTextRef.current));
        else closeRef.current();
      });
      if (!alive) { unlistenOutput?.(); unlistenExit?.(); return; }
      try {
        await bridgeApi.startProviderLogin(provider);
        launched = true;
        if (!alive && !exited) void bridgeApi.cancelProviderLogin(provider).catch(() => undefined);
      } catch {
        if (alive) setError(`${label}'s sign-in flow could not be started. Check that the CLI is installed.`);
      }
    })().catch(() => { if (alive) setError(`Bridge could not listen to ${label}'s sign-in flow. Close it and try again.`); });
    return () => {
      alive = false;
      unlistenOutput?.();
      unlistenExit?.();
      if (launched && !exited) void bridgeApi.cancelProviderLogin(provider).catch(() => undefined);
    };
  }, [provider, label]);
  useEffect(() => {
    outputRef.current?.scrollTo?.({ top: outputRef.current.scrollHeight });
  }, [output]);
  const writeReply = (data: string) => bridgeApi.writeTerminal("provider-login", provider, data).catch(() => {
    setError(`Bridge could not send your response to ${label}. Close sign-in and try again.`);
  });
  const send = () => {
    if (!entry.trim()) return;
    void writeReply(`${entry}\r`);
    setEntry("");
  };
  // Cancel abandons the sign-in, so the vendor process has to go with it.
  // `start_provider_login` reattaches to a live runtime without replaying the
  // URL and prompts this pane already dropped, so leaving it running would
  // hand the next attempt an empty terminal.
  const cancel = () => {
    void bridgeApi.cancelProviderLogin(provider).catch(() => undefined);
    onClose();
  };
  const cleanOutput = plainProviderLoginOutput(output);
  const loginUrl = providerLoginUrl(output);
  const loginCode = providerLoginCode(output);
  const replyField = <div className="flex gap-1.5">
    <input
      value={entry}
      onChange={event => setEntry(event.target.value)}
      onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); send(); } }}
      placeholder={bare ? "Paste code" : "Code or response"}
      aria-label={`Reply to the ${label} sign-in prompt`}
      className="min-w-0 flex-1 rounded-md border border-border bg-card px-2 py-1 font-mono text-caption text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
    />
    <button type="button" onClick={send} className="rounded-md border border-border px-2 py-1 text-caption font-medium text-foreground transition-colors hover:bg-accent">Send</button>
  </div>;
  const optionKeys = provider === "opencode" && <div className="mt-2 flex flex-wrap gap-2">{([
    ["Previous option", "\u001b[A"], ["Next option", "\u001b[B"], ["Choose option", "\r"],
  ] as const).map(([name, key]) => <button key={name} type="button" onClick={() => void writeReply(key)} className="min-h-8 rounded-md border border-border px-2 text-xs text-foreground">{name}</button>)}</div>;
  const outputPane = <pre ref={outputRef} aria-live="polite" aria-label={`${label} sign-in output`} className="max-h-36 overflow-y-auto whitespace-pre-wrap rounded-md bg-muted/40 p-2 font-mono text-[11px] leading-relaxed text-foreground">{cleanOutput || "Starting secure sign-in…"}</pre>;
  if (bare) return <div className="grid gap-4" aria-label={`${label} sign-in`}>
    {error ? <div role="alert" className="rounded-lg border border-destructive/25 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">{error}</div> : <div className="grid gap-3 rounded-xl border border-border-card bg-background/50 p-4">
      <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
        <LoaderCircle className="shrink-0 animate-spin" size={14} aria-hidden="true" />
        <span>{loginUrl ? "Waiting for you to finish in the browser" : `Starting ${label} sign-in…`}</span>
      </div>
      {loginUrl && <a href={loginUrl} target="_blank" rel="noreferrer" className="inline-flex min-h-9 items-center justify-center gap-2 rounded-lg bg-primary px-3 text-[13px] font-medium text-primary-foreground transition-colors hover:bg-primary/90">
        Open sign-in page <ExternalLink size={13} aria-hidden="true" />
      </a>}
      {loginCode && <p className="text-[12px] text-muted-foreground">Enter this code on the sign-in page: <code aria-label="One-time code" className="select-all rounded-md border border-border bg-card px-1.5 py-0.5 font-mono text-[13px] font-medium text-foreground">{loginCode}</code></p>}
      {loginUrl && !loginCode && <div className="grid gap-1.5">
        <p className="text-[11px] text-muted-foreground">If the page shows a code, paste it here.</p>
        {replyField}
      </div>}
    </div>}
    <details open={provider === "opencode" || (!!cleanOutput && !loginUrl)} className="group">
      <summary className="flex cursor-pointer list-none items-center gap-1.5 text-[11px] text-muted-foreground transition-colors hover:text-foreground">
        <Terminal size={12} aria-hidden="true" /> {loginUrl ? "Show details" : "Agent sign-in instructions"}
        <ChevronDown className="transition-transform group-open:rotate-180" size={12} aria-hidden="true" />
      </summary>
      <div className="mt-2 grid gap-2">
        {outputPane}
        {optionKeys}
        {(!loginUrl || loginCode) && replyField}
      </div>
    </details>
  </div>;
  return <section className="u-glass-soft mt-3 overflow-hidden rounded-xl border border-border-card" aria-label={`${label} sign-in`}>
    <header className="flex items-center gap-3 border-b border-border-card px-4 py-3">
      <span className="grid size-9 shrink-0 place-items-center rounded-xl border border-border-card bg-background text-foreground">
        <ShieldCheck size={17} aria-hidden="true" />
      </span>
      <div className="min-w-0 flex-1">
        <h3 className="text-[13px] font-medium text-foreground">Connect {label}</h3>
        <p className="mt-0.5 text-[11px] text-muted-foreground">{label} handles authentication; Bridge does not store your credentials.</p>
      </div>
      <button type="button" onClick={cancel} className="min-h-7 rounded-lg px-2.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Cancel</button>
    </header>
    <div className="grid gap-3 px-4 py-4">
      {error ? <div role="alert" className="rounded-lg border border-destructive/25 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">{error}</div> : <div className="flex items-start gap-3">
        <LoaderCircle className="mt-0.5 shrink-0 animate-spin text-muted-foreground" size={16} aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="text-[13px] font-medium text-foreground">{loginUrl ? "Finish in your browser" : `Follow ${label}'s sign-in instructions`}</p>
          <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">{loginUrl ? "Open the sign-in page below. When the agent finishes, Bridge checks its sign-in status again." : `Some agents first ask you to choose an account or provider below. A browser link appears when the agent supplies one.`}</p>
          {loginCode && <p className="mt-3 text-[12px] text-muted-foreground">Enter this code on the sign-in page: <code aria-label="One-time code" className="select-all rounded-md border border-border bg-card px-1.5 py-0.5 font-mono text-[13px] font-medium text-foreground">{loginCode}</code></p>}
          {loginUrl && <a href={loginUrl} target="_blank" rel="noreferrer" className="mt-3 inline-flex min-h-8 items-center gap-2 rounded-lg bg-primary px-3 text-[12px] font-medium text-primary-foreground transition-colors hover:bg-primary/90">
            Open sign-in page <ExternalLink size={12} aria-hidden="true" />
          </a>}
        </div>
      </div>}

      <details open={provider === "opencode" || (!!cleanOutput && !loginUrl)} className="group rounded-lg border border-border-card bg-background/60">
        <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground">
          <Terminal size={13} aria-hidden="true" /> {loginUrl ? "Sign-in details" : "Agent sign-in instructions"}
          <ChevronDown className="ml-auto transition-transform group-open:rotate-180" size={13} aria-hidden="true" />
        </summary>
        <div className="border-t border-border-card p-2.5">
          {outputPane}
          {optionKeys}
          <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">Only use this field when {label} asks for a code or response.</p>
          <div className="mt-2">{replyField}</div>
        </div>
      </details>
    </div>
  </section>;
}
