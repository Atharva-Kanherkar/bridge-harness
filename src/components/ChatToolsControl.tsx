import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronDown, Lock, RotateCw, Wrench, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { MenuPanel, MenuSeparator, useMenuPanel } from "@/components/ui/menu-panel";
import { Skeleton, SkeletonRows } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { bridgeApi } from "../api";
import { baseChoice, fits, overlay, removed, switched, type Choice } from "../chatToolChoice";
import { relativeTime } from "../connectorSurface";
import { formatTokens } from "../contextBreakdown";
import { errorMessage } from "../errors";
import { normalizeAccount } from "../providerAccounts";
import type { ChatToolItem, ChatTools, ChatToolSelection, ChatToolsDraft } from "../types";

/**
 * The connectors and skills a chat starts with, beside the access control.
 *
 * A choice only leaves items out, so the chip counts what is off. A chat saves
 * each switch through the shared busy guard and applies it at its next turn. A
 * busy chat refuses and keeps its old choice. A draft has no chat yet, so its
 * choice stays with the caller and travels inside the create of the chat.
 */
export type ChatToolsControlProps = { disabled?: boolean } & (
  | { sessionId: string; harness: string; account?: string | null; draft?: undefined }
  | { draft: ChatToolsDraft; choice: ChatToolSelection | null; onChoiceChange: (choice: ChatToolSelection | null) => void; sessionId?: undefined }
);

type Notice = { tone: "saved" | "refusal" | "error"; text: string };
/** A server and a skill can share a name, so a row is its kind and its id. */
type RowId = { kind: string; id: string };
type Pending = RowId & { on: boolean | null };
const sameRow = (a: RowId, b: RowId) => a.kind === b.kind && a.id === b.id;

const HARNESS_LABEL: Record<string, string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode" };
const SECTIONS = [
  { title: "Connectors", kinds: ["server", "apps", "app"] },
  { title: "Skills", kinds: ["skill", "allSkills"] },
  { title: "Plugins", kinds: ["plugin"] },
] as const;

const describeTarget = (harness: string, account: string | null) =>
  `${HARNESS_LABEL[harness] ?? harness} (${account ? `account ${account}` : "own login"})`;

/** The items of one section, in the order the host lists them, under their groups. */
function grouped(items: ChatToolItem[]): [string, ChatToolItem[]][] {
  const groups = new Map<string, ChatToolItem[]>();
  for (const item of items) groups.set(item.group, [...groups.get(item.group) ?? [], item]);
  return [...groups];
}

export function ChatToolsControl(props: ChatToolsControlProps) {
  const { disabled } = props;
  const menu = useMenuPanel<HTMLButtonElement>({ width: 336, height: 480 });
  const sessionId = props.sessionId;
  const draftHarness = props.draft?.harness;
  const draftAccount = props.draft?.account ?? null;
  const draftWorkspace = props.draft?.workspaceId ?? null;
  const key = sessionId
    ? `chat:${sessionId}:${props.harness}:${props.account ?? ""}`
    : `draft:${draftHarness}:${draftAccount ?? ""}:${draftWorkspace ?? ""}`;
  const [view, setView] = useState<{ key: string; data?: ChatTools; error?: string }>({ key });
  const [pending, setPending] = useState<Pending | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const generation = useRef(0);
  // The read in flight of each target. A read that a save can outdate is not shared.
  const inFlight = useRef(new Map<string, { read: Promise<ChatTools>; shared: boolean }>());
  // The save in flight of each target, so a return to a chat shows its save.
  const saves = useRef(new Map<string, Pending>());
  // The target on screen, or null after the control unmounts.
  const shown = useRef<string | null>(null);
  const refocus = useRef<RowId | null>(null);
  const refocusFrame = useRef<number | null>(null);

  const read = useCallback((fresh: boolean): Promise<ChatTools> => {
    const prior = inFlight.current.get(key);
    if (prior?.shared && !fresh) return prior.read;
    const params = sessionId ? { sessionId } : { draft: { harness: draftHarness ?? "", account: draftAccount, workspaceId: draftWorkspace } };
    const start = () => bridgeApi.chatTools(params);
    // A warm Codex read sends three requests under a limit of four for each chat, so
    // a new read of the same target starts when the read in flight ends.
    const entry = { read: prior ? prior.read.then(start, start) : start(), shared: !saves.current.has(key) };
    const release = () => { if (inFlight.current.get(key) === entry) inFlight.current.delete(key); };
    entry.read.then(release, release);
    inFlight.current.set(key, entry);
    return entry.read;
  }, [key, sessionId, draftHarness, draftAccount, draftWorkspace]);

  const load = useCallback(async (fresh = false) => {
    const ticket = ++generation.current;
    try {
      const data = await read(fresh);
      if (ticket === generation.current) setView({ key, data });
    } catch (error) {
      // A failed read keeps the last list of this chat on screen.
      if (ticket === generation.current) setView(current => ({ key, data: current.key === key ? current.data : undefined, error: errorMessage(error) }));
    }
  }, [key, read]);

  useEffect(() => {
    shown.current = key;
    setPending(saves.current.get(key) ?? null);
    setNotice(null);
    refocus.current = null;
    void load();
    return () => {
      // A late answer for the old target, or after an unmount, changes no state.
      shown.current = null;
      generation.current += 1;
    };
  }, [key, load]);

  // The menu gives the focus to its trigger after a click on an item, but a switch leaves
  // the menu open. The switch takes the focus back once it is enabled again.
  const { panelRef, triggerRef } = menu;
  const restoreFocus = useCallback(() => {
    const row = refocus.current;
    if (row === null) return;
    const panel = panelRef.current;
    const target = [...panel?.querySelectorAll<HTMLElement>("[data-tool-item]") ?? []]
      .find(element => sameRow({ kind: element.dataset.toolKind ?? "", id: element.dataset.toolItem ?? "" }, row))
      ?.querySelector<HTMLButtonElement>('[role="menuitemcheckbox"]');
    if (panel && target?.disabled) return;
    refocus.current = null;
    const active = document.activeElement;
    if (target && (active === triggerRef.current || active === document.body || panel?.contains(active))) target.focus();
  }, [panelRef, triggerRef]);
  const requestFocus = (row: RowId) => {
    refocus.current = row;
    if (refocusFrame.current !== null) window.cancelAnimationFrame(refocusFrame.current);
    refocusFrame.current = window.requestAnimationFrame(() => {
      refocusFrame.current = null;
      restoreFocus();
    });
  };
  useEffect(() => {
    if (!pending) restoreFocus();
  }, [pending, restoreFocus]);
  // A focus request ends when the menu closes or the control unmounts.
  useEffect(() => {
    if (!menu.open) return;
    return () => {
      refocus.current = null;
      if (refocusFrame.current !== null) window.cancelAnimationFrame(refocusFrame.current);
      refocusFrame.current = null;
    };
  }, [menu.open]);

  const data = view.key === key ? view.data : undefined;
  const loadError = view.key === key ? view.error : undefined;
  const saved = props.draft ? props.choice : data?.choice ?? null;
  // A draft knows its own target, so it finds a choice for another target with no read.
  const draftTarget = props.draft && saved && !fits(saved, props.draft.harness, normalizeAccount(props.draft.harness, draftAccount))
    ? `Chosen for ${describeTarget(saved.harness, normalizeAccount(saved.harness, saved.account))}, not applied`
    : null;
  const unitless = !!data && data.items.length === 0 && data.sources.length === 0;
  // A harness with no units lists nothing and reads nothing. The create refuses a draft
  // choice for another target, so that choice stays in view until the person clears it.
  if (unitless && !draftTarget) return null;

  const account = data ? normalizeAccount(data.harness, data.account) : null;
  const own = !!data && fits(saved, data.harness, account);
  const base = data ? baseChoice(saved, data.harness, account) : null;
  // The host reads a draft without its choice, so the local choice sets the states. The
  // host also sets the states from a choice of another account, which the next launch ignores.
  const local = !!props.draft || (!!saved && !own);
  const items = !data || !base ? [] : local ? overlay(data.items.filter(entry => entry.seen), base) : data.items;
  const offCount = items.filter(item => item.state === "off").length;
  const notApplied = draftTarget ?? (data && !data.selection.applies ? data.selection.notAppliedReason ?? "Not applied" : null);
  const choiceNotApplied = !!notApplied && !!saved;

  const change = async (next: Choice, item: ChatToolItem, on: boolean | null) => {
    if (!data) return;
    setNotice(null);
    if (props.draft) {
      props.onChoiceChange(next);
      return;
    }
    if (!sessionId) return;
    // The answer of a save reaches the state only while its chat is on screen, also
    // after the person left the chat and came back to it.
    const target = key;
    const here = () => shown.current === target;
    const mine: Pending = { kind: item.kind, id: item.id, on };
    const end = () => { if (saves.current.get(target) === mine) saves.current.delete(target); };
    // A save outdates each read that started before it, and no later read shares one.
    ++generation.current;
    const prior = inFlight.current.get(target);
    if (prior) prior.shared = false;
    saves.current.set(target, mine);
    setPending(mine);
    try {
      const outcome = await bridgeApi.updateChatTools({ sessionId, expectedRevision: data.selection.revision, selection: next });
      if (!here()) return end();
      if (outcome.refusal) {
        end();
        setPending(null);
        setNotice({ tone: "refusal", text: outcome.refusal });
        return;
      }
      const ticket = ++generation.current;
      const fresh = await read(true).catch(() => null);
      end();
      if (!here()) return;
      setPending(null);
      // The saved choice belongs to the target of the chat, so it applies.
      if (ticket === generation.current) setView({ key: target, data: fresh ?? {
        ...data,
        choice: next,
        items: overlay(data.items.filter(entry => entry.seen), next),
        selection: { revision: outcome.revision ?? data.selection.revision + 1, harness: next.harness, account: next.account, applies: true, notAppliedReason: null },
      } });
      setNotice({ tone: "saved", text: "Saved. The change applies at the next turn." });
    } catch (error) {
      end();
      if (!here()) return;
      setPending(null);
      setNotice({ tone: "error", text: errorMessage(error) });
      // A stale revision means another window saved first: show its choice.
      void load(true);
    }
  };

  const loading = !data && !loadError;
  const chipLabel = loading ? "Tools: loading" : choiceNotApplied ? "Tools: not applied" : offCount ? `Tools: ${offCount} left out` : "Tools: all on";
  const toggle = () => {
    if (!menu.open) {
      setNotice(null);
      if (!pending) void load();
    }
    menu.toggle();
  };

  return <>
    <button
      ref={menu.triggerRef}
      type="button"
      disabled={disabled}
      onClick={toggle}
      aria-haspopup="menu"
      aria-expanded={menu.open}
      aria-busy={loading || undefined}
      aria-label={chipLabel}
      title={notApplied ?? (offCount ? `${offCount} left out of this chat` : "The connectors and skills of this chat")}
      className="flex h-8 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground transition-colors enabled:hover:bg-accent disabled:cursor-default disabled:opacity-60"
    >
      <Wrench size={13} aria-hidden="true" className="shrink-0" />
      <span className="whitespace-nowrap">Tools</span>
      {loading
        ? <Skeleton className="h-2.5 w-8" />
        : choiceNotApplied
        ? <span className="whitespace-nowrap text-warning">not applied</span>
        : offCount > 0 && <span className="whitespace-nowrap rounded-full bg-accent px-1.5 tabular-nums text-foreground">{offCount} off</span>}
      <ChevronDown size={14} className={cn("shrink-0 transition-transform", menu.open && "rotate-180")} aria-hidden="true" />
    </button>
    <MenuPanel controller={menu} label="Tools of this chat">
      <div className="px-2 pb-1.5 pt-1">
        <p className="text-[13px] font-medium text-foreground">Tools of this chat</p>
        <p className="text-[11px] leading-4 text-muted-foreground">
          {props.draft ? "Leave out what this chat does not need. The chat starts with this choice." : "Leave out what this chat does not need. A change applies at the next turn."}
        </p>
      </div>
      {notApplied && <div className="mx-1 mb-1 rounded-md bg-accent px-2 py-1.5">
        <p role="note" className="text-[11px] leading-4 text-warning">{notApplied}</p>
        {/* The create refuses a draft choice for another target, so the draft can drop it. */}
        {props.draft && draftTarget && <button
          type="button"
          role="menuitem"
          tabIndex={-1}
          disabled={disabled}
          onClick={() => { setNotice(null); props.onChoiceChange(null); }}
          className="mt-1 flex h-7 items-center gap-1.5 rounded-md px-1.5 text-[12px] text-foreground transition-colors hover:bg-background/60 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-40"
        ><X size={12} aria-hidden="true" />Clear this choice</button>}
      </div>}
      {notice && <p
        role={notice.tone === "saved" ? "status" : "alert"}
        className={cn("mx-1 mb-1 px-1 text-[11px] leading-4", notice.tone === "error" ? "text-destructive" : notice.tone === "refusal" ? "text-warning" : "text-muted-foreground")}
      >{notice.text}</p>}
      {loading && <SkeletonRows label="Loading tools" rows={4} meta={false} trailing framed={false} rowClassName="min-h-8 px-2 py-1.5" />}
      {!data && loadError && <div className="px-2 py-1.5">
        <p role="alert" className="text-[11px] leading-4 text-destructive">{loadError}</p>
        <button type="button" role="menuitem" tabIndex={-1} onClick={() => void load()} className="mt-1 flex h-7 items-center gap-1.5 rounded-md px-1.5 text-[12px] text-foreground transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
          <RotateCw size={12} aria-hidden="true" />Try again
        </button>
      </div>}
      {data && base && <>
        {!items.length && <p className="px-2 py-1.5 text-[12px] text-muted-foreground">
          {unitless ? data.selection.notAppliedReason ?? "Nothing to list yet." : "Nothing to list yet."}
        </p>}
        {SECTIONS.map(section => {
          const members = items.filter(item => (section.kinds as readonly string[]).includes(item.kind));
          if (!members.length) return null;
          const groups = grouped(members);
          return <div key={section.title} role="group" aria-label={section.title} className="pb-1">
            <p className="px-2 pb-0.5 pt-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">{section.title}</p>
            {groups.map(([group, entries]) => <div key={group}>
              {groups.length > 1 && <p className="px-2 pt-1 text-[11px] text-faint">{group}</p>}
              {entries.map(item => <ToolRow
                key={`${item.kind}:${item.id}`}
                item={item}
                pending={pending && sameRow(pending, item) ? pending : null}
                busy={!!pending || !!disabled}
                onSwitch={on => {
                  requestFocus(item);
                  void change(switched(base, item, on, items), item, on);
                }}
                onRemove={() => void change(removed(base, item), item, null)}
              />)}
            </div>)}
          </div>;
        })}
        {data.sources.length > 0 && <>
          <MenuSeparator />
          <div className="px-2 pb-1 pt-0.5 text-[11px] leading-4 text-muted-foreground">
            <p className="font-medium">Read from</p>
            {data.sources.map(source => <p key={`${source.kind}:${source.label}`} className={cn(!source.available && "text-faint")}>
              {source.label}{source.available && source.at ? ` · ${relativeTime(source.at)}` : ""}
              {source.note ? <span className="block text-faint">{source.note}</span> : null}
            </p>)}
          </div>
        </>}
      </>}
    </MenuPanel>
  </>;
}

function figureText(item: ChatToolItem): string | null {
  return item.tokens ? `${formatTokens(item.tokens.value)} tokens · ${item.tokens.source}` : null;
}

function ToolRow({ item, pending, busy, onSwitch, onRemove }: {
  item: ChatToolItem;
  pending: Pending | null;
  busy: boolean;
  onSwitch: (on: boolean) => void;
  onRemove: () => void;
}) {
  const figure = figureText(item);
  const remove = !item.seen && <button
    type="button"
    role="menuitem"
    tabIndex={-1}
    disabled={busy}
    onClick={onRemove}
    aria-label={`Remove ${item.label} from the choice`}
    title="Remove from the choice"
    className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-40"
  >{pending?.on === null ? <Spinner className="size-3.5" /> : <X size={13} aria-hidden="true" />}</button>;
  const meta = <>
    {item.fixedReason && <span className="block text-[11px] leading-4 text-muted-foreground">{item.fixedReason}</span>}
    {figure && <span title={figure} className="block truncate text-[11px] leading-4 text-muted-foreground">{figure}</span>}
    {!item.seen && <span className="block text-[11px] leading-4 text-faint">Saved, not listed now</span>}
  </>;
  if (item.state === "fixed") {
    return <div data-tool-kind={item.kind} data-tool-item={item.id} className="flex items-start gap-2 rounded-md px-2 py-1.5">
      <Lock size={12} aria-hidden="true" className="mt-0.5 shrink-0 text-faint" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] text-foreground">{item.label}</span>
        {meta}
      </span>
      {remove}
    </div>;
  }
  const on = pending?.on ?? item.state === "on";
  return <div data-tool-kind={item.kind} data-tool-item={item.id} className="flex items-center gap-1">
    <button
      type="button"
      role="menuitemcheckbox"
      tabIndex={-1}
      aria-checked={on}
      disabled={busy}
      onClick={() => onSwitch(!on)}
      className="flex min-h-7 min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1 text-left transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-default disabled:hover:bg-transparent"
    >
      <span className="min-w-0 flex-1">
        <span className={cn("block truncate text-[13px]", on ? "text-foreground" : "text-muted-foreground")}>{item.label}</span>
        {meta}
      </span>
      {pending?.on != null && <Spinner className="size-3.5 shrink-0 text-muted-foreground" />}
      <span aria-hidden="true" className={cn("relative h-[18px] w-8 shrink-0 rounded-full transition-colors", on ? "bg-foreground" : "bg-foreground/15", busy && "opacity-60")}>
        <span className={cn("absolute top-[2px] size-[14px] rounded-full bg-background transition-[left] duration-150", on ? "left-4" : "left-[2px]")} />
      </span>
    </button>
    {remove}
  </div>;
}
