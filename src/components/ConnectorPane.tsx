import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AtSign,
  ChevronRight,
  CornerUpLeft,
  ExternalLink,
  Inbox,
  LoaderCircle,
  MessageSquare,
  PlugZap,
  RefreshCw,
  Send,
  ShieldAlert,
  Sparkles,
  TriangleAlert,
  X,
} from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  emptyStateFor,
  inboxFamilies,
  isDegraded,
  itemSubtitle,
  primaryConnector,
  unreadCount,
} from "../connectorSurface";
import type {
  ConnectorActionRequest,
  ConnectorCardBlock,
  ConnectorDescriptor,
  ConnectorInboxItem,
  ConnectorInboxResult,
} from "../protocol/generated/protocol";

// The connector inbox: a dock pane that renders one notification at a time and
// lets you answer it without leaving Bridge.
//
// Two rules shape the whole component:
//
// 1. **A card is drawn, never injected.** `card.blocks` is a closed set of typed
//    blocks, so a Slack message reaches the DOM as text in a `<p>` and can never
//    be markup. There is no iframe and no `dangerouslySetInnerHTML` here, which
//    is why there is also no sandbox to get wrong.
// 2. **Approval is Bridge's own chrome.** Sending asks the host, which refuses
//    the first call and hands back the literal effect; the sheet below renders
//    that sentence. The card cannot draw its own approve button, because the
//    card cannot draw anything.

export function ConnectorPane({ visible = true, focusItemKey, onClose, onUnreadChange }: {
  /** False while another dock pane is showing; the pane stays mounted. */
  visible?: boolean;
  /** Deep-link target from a toast. */
  focusItemKey?: string;
  onClose?: () => void;
  onUnreadChange?: (count: number) => void;
}) {
  const [inbox, setInbox] = useState<ConnectorInboxResult>();
  const [connectors, setConnectors] = useState<ConnectorDescriptor[]>([]);
  const [selectedKey, setSelectedKey] = useState<string>();
  const [draft, setDraft] = useState("");
  const [pendingApproval, setPendingApproval] = useState<{ effect: string; action: ConnectorActionRequest }>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const refresh = useCallback(async () => {
    const [next, list] = await Promise.all([bridgeApi.connectorInbox(), bridgeApi.connectorList()]);
    setInbox(next);
    setConnectors(list.connectors);
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => {
    if (!visible) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void bridgeApi.onConnectorInboxChanged(() => { void refresh(); }).then(stop => {
      if (cancelled) stop();
      else unlisten = stop;
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [visible, refresh]);
  useEffect(() => {
    if (focusItemKey) setSelectedKey(focusItemKey);
  }, [focusItemKey]);
  useEffect(() => {
    onUnreadChange?.(inbox ? unreadCount(inbox.items) : 0);
  }, [inbox, onUnreadChange]);

  const items = inbox?.items ?? [];
  const selected = useMemo(
    () => items.find(item => item.itemKey === selectedKey) ?? items.find(item => item.state !== "resolved"),
    [items, selectedKey],
  );
  const connector = primaryConnector(connectors);
  const empty = inbox ? emptyStateFor(inbox) : null;
  const degraded = inbox ? isDegraded(inbox.poll, connector?.family) : false;

  const act = async (action: ConnectorActionRequest, approved?: boolean) => {
    if (!selected) return;
    setBusy(true);
    setError(undefined);
    try {
      const result = await bridgeApi.connectorAct(selected.itemKey, action, approved);
      if (result.status === "approvalRequired") {
        // The host refused and told us exactly what would happen. That sentence
        // is the approval — we never compose our own.
        setPendingApproval({ effect: result.effect, action });
        return;
      }
      setPendingApproval(undefined);
      if (result.status === "refused") setError(result.reason);
      else setDraft("");
      await refresh();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };

  return <div className="relative flex h-full min-w-0 flex-col overflow-hidden bg-background">
    <header className="flex min-h-14 shrink-0 items-center gap-2 border-b border-border bg-muted/25 px-3 py-2">
      <Inbox size={15} aria-hidden="true" className="shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[11px] font-medium text-foreground">Inbox</div>
        <div className="truncate text-[11px] text-muted-foreground">
          {connector?.available ? `${connector.displayName} · via your harness` : "No connector available"}
        </div>
      </div>
      {!!items.filter(item => item.state !== "resolved").length && <Badge variant="secondary" size="sm">
        {items.filter(item => item.state !== "resolved").length} unread
      </Badge>}
      {degraded && <Badge variant="warning" size="sm">Degraded</Badge>}
      <Button
        variant="ghost"
        size="icon-xs"
        loading={busy}
        aria-label="Check for new messages"
        onClick={() => void (async () => {
          setBusy(true);
          try {
            // Refresh every family this build has an inbox for, not a named one.
            await Promise.all(inboxFamilies(connectors).map(family => bridgeApi.connectorRefresh(family)));
            await refresh();
          }
          finally { setBusy(false); }
        })()}
      >
        <RefreshCw size={12} />
      </Button>
      {onClose && <Button variant="ghost" size="icon-xs" onClick={onClose} aria-label="Close Inbox"><ChevronRight size={14} /></Button>}
    </header>

    {connector && !connector.available && <ConnectorHint connector={connector} />}

    {!inbox ? <div className="grid flex-1 place-items-center text-muted-foreground"><LoaderCircle className="animate-spin" size={18} /></div>
      : empty ? <EmptyState state={empty} />
      : <div className="flex min-h-0 flex-1 flex-col">
        <ul className="max-h-[38%] shrink-0 overflow-y-auto border-b border-border" aria-label="Unread messages">
          {items.map((item, index) => <li key={item.itemKey}>
            <ItemRow
              item={item}
              index={index}
              active={item.itemKey === selected?.itemKey}
              onSelect={() => { setSelectedKey(item.itemKey); setDraft(""); setError(undefined); }}
            />
          </li>)}
        </ul>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {selected ? <CardView item={selected} /> : null}
        </div>
        {selected && selected.state !== "resolved" && <Composer
          item={selected}
          draft={draft}
          busy={busy}
          error={error}
          onDraft={setDraft}
          onSend={() => void act({ kind: "reply", text: draft })}
          onReact={emoji => void act({ kind: "react", emoji })}
          onDismiss={() => void (async () => { await bridgeApi.connectorDismiss(selected.itemKey); await refresh(); })()}
        />}
      </div>}

    {pendingApproval && <ApprovalSheet
      effect={pendingApproval.effect}
      busy={busy}
      onDeny={() => void act(pendingApproval.action, false)}
      onApprove={() => void act(pendingApproval.action, true)}
    />}
  </div>;
}

function ConnectorHint({ connector }: { connector: ConnectorDescriptor }) {
  return <div className="flex items-start gap-2 border-b border-border bg-muted/20 px-3 py-2 text-[11px] leading-4 text-muted-foreground">
    <PlugZap size={13} aria-hidden="true" className="mt-0.5 shrink-0" />
    <span>{connector.explanation ?? `${connector.displayName} is not available.`}</span>
  </div>;
}

function EmptyState({ state }: { state: NonNullable<ReturnType<typeof emptyStateFor>> }) {
  const Icon = state.tone === "degraded" ? TriangleAlert : state.tone === "waiting" ? LoaderCircle : Inbox;
  return <div className="grid flex-1 place-items-center p-6 text-center">
    <div className="max-w-xs">
      <Icon
        size={22}
        aria-hidden="true"
        className={cn("mx-auto", state.tone === "degraded" ? "text-warning" : "text-muted-foreground", state.tone === "waiting" && "animate-spin")}
      />
      <h2 className="mt-3 font-display text-sm font-semibold text-foreground">{state.title}</h2>
      <p className="mt-1.5 text-[11px] leading-5 text-muted-foreground">{state.detail}</p>
    </div>
  </div>;
}

function ItemRow({ item, index, active, onSelect }: {
  item: ConnectorInboxItem;
  index: number;
  active: boolean;
  onSelect: () => void;
}) {
  const Icon = item.kind === "mention" ? AtSign : item.kind === "threadReply" ? CornerUpLeft : MessageSquare;
  const unresolved = item.state !== "resolved";
  return <button
    type="button"
    onClick={onSelect}
    aria-current={active ? "true" : undefined}
    style={{ "--connector-row-delay": `${Math.min(index, 8) * 40}ms` } as React.CSSProperties}
    className={cn(
      "connector-row-enter flex w-full items-start gap-2.5 px-3 py-2 text-left transition-colors",
      active ? "bg-selection text-selection-foreground" : "hover:bg-accent",
      !unresolved && "opacity-55",
    )}
  >
    <span className="relative mt-0.5 shrink-0">
      <Icon size={13} aria-hidden="true" className={active ? "" : "text-muted-foreground"} />
      {item.state === "pending" && <span
        aria-hidden="true"
        className="connector-dot-live absolute -right-1 -top-1 size-1.5 rounded-full bg-foreground"
      />}
    </span>
    <span className="min-w-0 flex-1">
      <span className={cn("block truncate text-[12px] font-medium", !active && "text-foreground")}>
        {item.card?.headline ?? `${item.author} · ${item.channelLabel}`}
      </span>
      <span className={cn("block truncate text-[11px]", active ? "opacity-80" : "text-muted-foreground")}>
        {itemSubtitle(item)}
      </span>
    </span>
    {item.resolution && <Badge variant="outline" size="sm" className="mt-0.5 shrink-0 capitalize">{item.resolution}</Badge>}
  </button>;
}

/**
 * Draw one card.
 *
 * Every branch below renders into a text node. `card.blocks` is a closed union
 * contracted in `bridge-protocol`, so there is no case where connector content
 * becomes markup — which is what lets this be a plain component instead of a
 * sandboxed frame.
 */
function CardView({ item }: { item: ConnectorInboxItem }) {
  const card = item.card;
  if (!card) return <PendingCard item={item} />;
  return <article key={card.itemKey} className="connector-settle space-y-3 p-3.5">
    <header className="space-y-1">
      <h2 className="font-display text-[15px] font-semibold leading-snug text-foreground">{card.headline}</h2>
      <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
        <span>{itemSubtitle(item)}</span>
        {card.harnessRendered && <Badge variant="outline" size="sm" className="gap-1">
          <Sparkles size={9} aria-hidden="true" />Rendered by your harness
        </Badge>}
        {item.permalink && <a
          href={item.permalink}
          target="_blank"
          rel="noreferrer noopener"
          className="inline-flex items-center gap-1 underline-offset-2 hover:underline"
        >
          Open in {item.family}<ExternalLink size={9} aria-hidden="true" />
        </a>}
      </div>
    </header>

    {item.renderRejection && <p className="flex items-start gap-1.5 rounded-lg bg-muted/60 px-2 py-1.5 text-[11px] leading-4 text-muted-foreground">
      <ShieldAlert size={12} aria-hidden="true" className="mt-0.5 shrink-0" />
      <span>Bridge wrote this card itself — {item.renderRejection}</span>
    </p>}

    <div className="space-y-2.5">
      {card.blocks.map((block, index) => <Block key={index} block={block} />)}
    </div>
  </article>;
}

/** A message's own clock time. The harness echoes back whatever timestamp the
 *  provider gave it, which is an ISO string — readable to a parser, not to a
 *  person glancing at a notification. */
function messageTime(value: string): string {
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) return value;
  return new Date(parsed).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function Block({ block }: { block: ConnectorCardBlock }) {
  switch (block.kind) {
    case "message":
      return <div className="u-glass-soft rounded-xl p-2.5">
        <div className="flex items-baseline gap-2">
          <b className="text-[12px] font-semibold text-foreground">{block.author}</b>
          {block.timestamp && <span className="text-[10px] text-muted-foreground">{messageTime(block.timestamp)}</span>}
        </div>
        {/* Untrusted third-party text, rendered as text. */}
        <p className="mt-1 whitespace-pre-wrap break-words text-[12.5px] leading-relaxed text-foreground">{block.text}</p>
      </div>;
    case "summary":
      return <p className="flex items-start gap-1.5 text-[12px] leading-relaxed text-muted-foreground">
        <Sparkles size={11} aria-hidden="true" className="mt-1 shrink-0" />
        <span>{block.text}</span>
      </p>;
    case "context":
      return <p className="text-[11px] leading-relaxed text-muted-foreground">{block.text}</p>;
    case "fact":
      return <div className="flex items-baseline gap-2 text-[11px]">
        <span className="shrink-0 uppercase tracking-wider text-muted-foreground">{block.label}</span>
        <span className="min-w-0 flex-1 truncate text-foreground">{block.value}</span>
      </div>;
    default:
      // A block kind this build does not know, from a newer host. Unreachable
      // through the type system and entirely reachable at runtime, which is the
      // only kind of unreachable that matters for data off a wire. Rendering the
      // block's own text keeps the card useful instead of silently dropping a
      // paragraph of it; the fields are still bounded by the host's validator.
      return <UnknownBlock block={block} />;
  }
}

/** Best-effort rendering of a block kind added after this build shipped. */
function UnknownBlock({ block }: { block: ConnectorCardBlock }) {
  const record = block as unknown as Record<string, unknown>;
  const text = ["text", "value", "label", "author"]
    .map(field => record[field])
    .filter((value): value is string => typeof value === "string" && value.trim() !== "")
    .join(" · ");
  if (!text) return null;
  return <p className="text-[12px] leading-relaxed text-muted-foreground">{text}</p>;
}

/**
 * What an item looks like between arriving and being rendered.
 *
 * The message is shown, not hidden: it already arrived, and a spinner where the
 * text should be would trade a real notification for a prettier one. Only the
 * headline shimmers, because only the headline is still coming.
 */
function PendingCard({ item }: { item: ConnectorInboxItem }) {
  return <article className="space-y-3 p-3.5">
    <header className="space-y-1">
      <h2 className="connector-shimmer font-display text-[15px] font-semibold leading-snug">
        {item.author} sent you a message
      </h2>
      <p className="text-[11px] text-muted-foreground">{itemSubtitle(item)}</p>
    </header>
    <div className="u-glass-soft rounded-xl p-2.5">
      <b className="text-[12px] font-semibold text-foreground">{item.author}</b>
      <p className="mt-1 whitespace-pre-wrap break-words text-[12.5px] leading-relaxed text-foreground">{item.text}</p>
    </div>
  </article>;
}

function Composer({ item, draft, busy, error, onDraft, onSend, onReact, onDismiss }: {
  item: ConnectorInboxItem;
  draft: string;
  busy: boolean;
  error?: string;
  onDraft: (value: string) => void;
  onSend: () => void;
  onReact: (emoji: string) => void;
  onDismiss: () => void;
}) {
  const suggestions = item.card?.suggestedReplies ?? [];
  const ready = !!item.card;
  return <div className="shrink-0 space-y-2 border-t border-border p-2.5">
    {!!suggestions.length && <div className="flex flex-wrap gap-1.5">
      {suggestions.map(reply => <button
        key={reply}
        type="button"
        onClick={() => onDraft(reply)}
        className="max-w-full truncate rounded-full border border-border px-2.5 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground active:scale-[0.98]"
      >
        {reply}
      </button>)}
    </div>}
    {error && <p className="flex items-start gap-1.5 text-[11px] leading-4 text-destructive">
      <TriangleAlert size={11} aria-hidden="true" className="mt-0.5 shrink-0" />{error}
    </p>}
    <div className="flex items-end gap-1.5">
      <textarea
        value={draft}
        disabled={!ready}
        onChange={event => onDraft(event.target.value)}
        onKeyDown={event => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && draft.trim()) onSend();
        }}
        rows={2}
        aria-label={`Reply to ${item.author}`}
        placeholder={ready ? `Reply to ${item.author}…` : "Waiting for the card…"}
        className="min-h-[3.25rem] min-w-0 flex-1 resize-none rounded-lg border border-input bg-card px-2 py-1.5 text-[12.5px] leading-relaxed outline-none transition-colors focus:border-ring disabled:opacity-50"
      />
      <div className="flex shrink-0 flex-col gap-1.5">
        <Button size="icon-xs" disabled={!ready || !draft.trim()} loading={busy} onClick={onSend} aria-label="Send reply">
          <Send size={12} />
        </Button>
        <Button variant="ghost" size="icon-xs" disabled={!ready} onClick={() => onReact("eyes")} aria-label="React with eyes">
          <span aria-hidden="true" className="text-[12px] leading-none">👀</span>
        </Button>
      </div>
    </div>
    <div className="flex items-center justify-between text-[10px] text-muted-foreground">
      <span>Sending asks you to confirm first.</span>
      <button type="button" onClick={onDismiss} className="underline-offset-2 hover:text-foreground hover:underline">
        Dismiss
      </button>
    </div>
  </div>;
}

/**
 * Bridge's own approval chrome.
 *
 * `effect` is the sentence the host composed and refused on — it names the
 * destination and quotes the literal text. Rendering the host's sentence rather
 * than one assembled here is what keeps the dialog honest about what the second
 * call will actually do.
 */
function ApprovalSheet({ effect, busy, onApprove, onDeny }: {
  effect: string;
  busy: boolean;
  onApprove: () => void;
  onDeny: () => void;
}) {
  return <div className="absolute inset-0 z-20 grid place-items-end bg-background/70 backdrop-blur-sm">
    <div role="dialog" aria-label="Confirm connector action" className="u-glass-popover connector-approval-in m-2.5 w-[calc(100%-1.25rem)] rounded-xl border border-border p-3.5 shadow-2xl">
      <div className="flex items-center gap-2 text-[13px] font-semibold text-foreground">
        <Send size={14} aria-hidden="true" className="shrink-0" />Send this?
      </div>
      <p className="mt-2 whitespace-pre-wrap break-words rounded-lg bg-muted/60 p-2 text-[12px] leading-relaxed text-foreground">
        {effect}
      </p>
      <div className="mt-3 flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onDeny}><X size={12} />Cancel</Button>
        <Button size="sm" loading={busy} onClick={onApprove}><Send size={12} />Send</Button>
      </div>
    </div>
  </div>;
}
