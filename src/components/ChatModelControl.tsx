import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { Skeleton, SkeletonRows } from "@/components/ui/skeleton";
import { Check, ChevronDown, RefreshCw, Search, Star } from "lucide-react";
import type { AdapterDescriptor, Harness, ModelOption } from "../types";
import { harnessLabel } from "../utils";
import { accountLabel, normalizeAccount, type AccountChoice } from "../providerAccounts";
import { HarnessMark } from "./harnessMarks";
import { isModelFavorite, useModelFavorites, type ModelFavorite } from "../modelFavorites";
import { effortIndex, effortLevelsFrom, effortMeaning } from "./effort/effortLevels";
import { EffortControl } from "./effort/EffortControl";
import { SpeedControl } from "./effort/SpeedControl";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RowButton } from "@/components/ui/row";
import { Tip } from "@/components/ui/tooltip";
import { useLatest } from "../useLatest";
import { useResetOnChange } from "../useResetOnChange";

// Vendor noise at the tail of a catalog label. OpenCode Zen's free tier
// suffixes "(Unlimited)": it names the plan, not the model. Cursor reports a
// model whose variant brackets came through empty ("default[]"), which read as
// a rendering bug in the pill. Both are noise wherever Bridge shows the label
// as a name — the pill, the picker rows, and modelDisplayName alike. Repeated
// so a label carrying both, in either order, comes out clean.
const LABEL_NOISE_SUFFIX = /(?:\s*(?:\(unlimited\)|\[\s*\]))+\s*$/i;

/** Display-only cleanup of a catalog label. Ids and what `onChange` emits are
 *  never touched — only the text Bridge renders. */
export function cleanModelLabel(label: string): string {
  const cleaned = label.replace(LABEL_NOISE_SUFFIX, "").trim();
  // A label that is nothing but noise still has to name something.
  return cleaned || label.trim();
}

/** Exact-match lookup of a session's configured model, for display outside
 *  the control itself (e.g. a tooltip or a plain-text mention). */
export function modelDisplayName(adapters: AdapterDescriptor[], harness: Harness, model?: string | null): string {
  const adapter = adapters.find(item => item.id === harness);
  const label = adapter?.models.find(option => option.id === model)?.label ?? model ?? "Automatic";
  return cleanModelLabel(label);
}

export type ChatModelControlProps = {
  adapters: AdapterDescriptor[];
  harness: Harness;
  model: string | null;
  disabled?: boolean;
  disabledReason?: string;
  /** `account` is the extra sign-in picked, or null for the user's own login.
   *  Only surfaces that pass `accounts` ever receive a non-null one. */
  onChange: (harness: Harness, model: string | null, account: string | null) => void;
  /** Extra Claude Code and Codex sign-ins, each offered as its own group
   *  beside the harness (T3 Code's provider instances). */
  accounts?: readonly AccountChoice[];
  /** The account the chat signs in with; null or the harness id is the
   *  user's own login. */
  account?: string | null;
  compact?: boolean;
  roleLabel?: string;
  /** Overrides the pill's width cap. Defaults to the composer's width; the
   *  session toolbar's tighter row passes a narrower cap. */
  maxWidthClassName?: string;
  /** Which way the picker opens. The composer sits at the bottom of the view,
   *  so it opens upward — the default. A caller in the session toolbar (the
   *  top row of a container that clips its overflow) must pass `"down"`, or
   *  the panel lands above the viewport and never becomes visible. */
  placement?: "up" | "down";
  /** Active reasoning effort for the session, checked in the picker's
   *  Thinking row. */
  effort?: string | null;
  /** Restricts the offered levels, e.g. to what a specialist worker can carry. */
  allowedEfforts?: readonly string[];
  /** Changes the reasoning effort. When omitted the footer control still shows
   *  the current effort but is inert — the state lives with the caller, so a
   *  surface that has not wired a setter yet reads rather than writes. */
  onEffortChange?: (effort: string) => void;
  /** The chat's speed tier: an id from the model's `speedTiers`, `standard`,
   *  or null for the provider's default. */
  speed?: string | null;
  /** Changes the speed. Without it the speed control is not shown: only a
   *  running chat has a speed to change. */
  onSpeedChange?: (speed: string) => void;
  /** Re-probes provider-owned catalogues and repaints from fresh descriptors. */
  onRefresh?: () => Promise<void> | void;
  /** The host is still resolving which model this chat will use (new-chat
   *  defaults, or a project switch). Until that settles, the pill says so
   *  instead of naming the fallback it happened to compute first. */
  loading?: boolean;
  /** A label for `model` from an earlier catalog read, so the pill can name the
   *  model before provider discovery has landed. */
  knownLabel?: string | null;
  /** The composer's access mode control, repeated in the picker's footer so
   *  model, thinking, speed and access are all in reach from one place. */
  accessControl?: ReactNode;
};

const NO_ACCOUNTS: readonly AccountChoice[] = [];

/** The popover's tallest and shortest useful heights, in CSS pixels. The
 *  shortest is what search, the harness rail, a few rows of the list, the
 *  Thinking row and the footer need together: below it the picker flips to
 *  the roomier side if there is one, and otherwise scrolls as a whole. */
const MAX_POPOVER_HEIGHT = 520;
const MIN_POPOVER_HEIGHT = 400;
const VIEWPORT_MARGIN = 12;
/** How long an unresolved model stays silent before the pill admits it is
 *  loading. Short enough to explain a slow read, long enough that a local one
 *  never flashes a spinner. */
const MODEL_LOADING_DELAY_MS = 600;

export function ChatModelControl({ adapters, harness, model, disabled, disabledReason, onChange, accounts = NO_ACCOUNTS, account, compact, roleLabel = "Chat", maxWidthClassName = "max-w-[220px]", placement = "up", effort, allowedEfforts, onEffortChange, speed, onSpeedChange, onRefresh, loading, knownLabel, accessControl }: ChatModelControlProps) {
  const currentAccount = normalizeAccount(harness, account);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  // Optimistic selection: the row the user just picked highlights and drives
  // the effort ladder at once, while the host is still switching. Cleared when
  // the props catch up or when the host's disable window ends.
  const [pending, setPending] = useState<{ harness: Harness; model: string | null; account: string | null } | null>(null);
  // The level the user last chose here, until the host confirms it. `sent`
  // is false while the host is still switching models — the level waits and
  // goes out the instant the switch settles, provided the pick landed. The
  // shown value is this one, so a round-trip never flashes the stale prop.
  const [queued, setQueued] = useState<{ value: string; sent: boolean } | null>(null);
  // True while a request the picker itself made — a model pick or a level —
  // is outstanding at the host. A live session's handler sets busy, which
  // flips `disabled` for the length of the request; that flip must not count
  // as "a turn started" and close the popover, must keep the effort control
  // live, and must block a second pick that would race the first on the
  // session lock. It is state of its own rather than "is there a pending
  // pick" because the props can confirm the new model while the host is still
  // busy — the highlight is settled then, the request is not. A disable from
  // anywhere else closes the popover and drops anything of ours in flight,
  // since the host is busy with something else now.
  const [inFlight, setInFlight] = useState(false);
  const wasDisabled = useRef(!!disabled);
  // The host's busy edge drives a small state machine that also hands a queued
  // effort to the host, so the transition and that call stay in one effect.
  useEffect(() => {
    const was = wasDisabled.current;
    wasDisabled.current = !!disabled;
    if (disabled && !was) {
      // oxlint-disable-next-line react/set-state-in-effect -- the busy edge is seen only after commit, beside the onEffortChange it may send
      if (!inFlight) { setOpen(false); setPending(null); setQueued(null); }
      return;
    }
    if (!disabled && was) {
      setInFlight(false);
      // The request settled. Only a pick the host adopted may carry the queue;
      // a failed switch must not apply the level to the model just left.
      const landed = !pending || (pending.harness === harness && pending.model === model && pending.account === currentAccount);
      setPending(null);
      if (!queued) return;
      if (!queued.sent) {
        if (landed && onEffortChange) {
          setInFlight(true);
          setQueued({ value: queued.value, sent: true });
          onEffortChange(queued.value);
        } else {
          setQueued(null);
        }
      } else if (effort !== queued.value) {
        // The host settled without adopting the level: show what it has.
        setQueued(null);
      }
    }
  }, [disabled, inFlight, pending, queued, harness, model, currentAccount, effort, onEffortChange]);
  // The props caught up with the optimistic state. A host that applied the
  // change without ever going busy (the Welcome draft) has nothing of ours
  // outstanding any more; one still busy is confirming early, and stays in
  // flight until it settles.
  // These run after the busy-edge effect above, in the same pass, so a queued
  // effort is handed over before the optimistic state is dropped. Moving them
  // into render would clear it first and the effort would never be sent.
  useEffect(() => {
    if (!pending || pending.harness !== harness || pending.model !== model || pending.account !== currentAccount) return;
    // oxlint-disable-next-line react/set-state-in-effect -- ordered after the busy-edge effect on purpose
    setPending(null);
    if (!disabled) setInFlight(false);
  }, [harness, model, currentAccount, pending, disabled]);
  useEffect(() => {
    if (!queued?.sent || effort !== queued.value) return;
    // oxlint-disable-next-line react/set-state-in-effect -- ordered after the busy-edge effect on purpose
    setQueued(null);
    if (!disabled) setInFlight(false);
  }, [effort, queued, disabled]);
  // Closing while nothing is in flight forgets the optimistic state. Closing
  // mid-request keeps it: the pick already went to the host, and a level chosen
  // meanwhile still goes out when the switch settles.
  useEffect(() => {
    // oxlint-disable-next-line react/set-state-in-effect -- ordered after the busy-edge effect on purpose
    if (!open && !disabled) { setPending(null); setQueued(null); }
  }, [open, disabled]);
  // Fit the popover to the window. The composer sits near the bottom of the
  // view, so an upward popover of fixed height ran off the top of a short
  // window; measure the room on each side, cap the height to it, and flip when
  // the requested side is too short and the other has more.
  const wrapper = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState<{ side: "up" | "down"; maxHeight: number } | null>(null);
  useResetOnChange([open], () => { if (!open) setFit(null); });
  useLayoutEffect(() => {
    if (!open) return;
    const anchor = wrapper.current;
    if (!anchor) return;
    const measure = () => {
      const rect = anchor.getBoundingClientRect();
      // No box (jsdom, detached) means nothing to fit to: keep the defaults.
      if (rect.width === 0 && rect.height === 0) { setFit(null); return; }
      let top = 0;
      let bottom = window.innerHeight;
      // A viewport-sized popover can still be hidden behind a toolbar or a
      // scrolling pane. Intersect every clipping ancestor before choosing a side.
      for (let parent = anchor.parentElement; parent; parent = parent.parentElement) {
        const overflow = getComputedStyle(parent);
        if (/(auto|scroll|hidden|clip)/.test(overflow.overflowY || overflow.overflow)) {
          const bounds = parent.getBoundingClientRect();
          top = Math.max(top, bounds.top);
          bottom = Math.min(bottom, bounds.bottom);
        }
      }
      const above = Math.max(0, rect.top - top - VIEWPORT_MARGIN);
      const below = Math.max(0, bottom - rect.bottom - VIEWPORT_MARGIN);
      let side = placement;
      const room = side === "up" ? above : below;
      const other = side === "up" ? below : above;
      if (room < MIN_POPOVER_HEIGHT && other > room) side = side === "up" ? "down" : "up";
      // The preferred minimum is a reason to flip, never a reason to overflow.
      setFit({ side, maxHeight: Math.min(MAX_POPOVER_HEIGHT, side === "up" ? above : below) });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(anchor);
    for (let parent = anchor.parentElement; parent; parent = parent.parentElement) observer?.observe(parent);
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [open, placement]);
  const side = fit?.side ?? placement;
  // Too short for the preferred minimum. The list keeps its floor either way
  // (it used to be the only part allowed to shrink, so a short window crushed
  // it to a sliver under the Thinking row); here the switch caption also
  // gives up its line, and whatever still does not fit scrolls with the
  // popover as a whole.
  const tight = !!fit && fit.maxHeight < MIN_POPOVER_HEIGHT;
  const dialogRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  // Closing hands focus back to the composer the picker belongs to, so typing
  // carries on. Only when focus was inside the picker: a close the user did
  // not drive from here must not steal it. Outside a composer (the aside's
  // header) focus goes back to the pill that opened it.
  const closePicker = () => {
    const restore = !!dialogRef.current?.contains(document.activeElement);
    setOpen(false);
    if (!restore) return;
    const anchor = wrapper.current;
    const composer = anchor?.closest("[data-composer-frame]")?.querySelector<HTMLTextAreaElement>("textarea:not([disabled])");
    (composer ?? anchor?.querySelector<HTMLButtonElement>("button:not([disabled])"))?.focus();
  };
  // Escape closes the picker and nothing else. Captured on window so it wins
  // against modal hosts with their own window-level Escape (the aside panel
  // closes itself on Escape — without this, dismissing the picker tore down
  // the whole aside). A menu the footer portalled out (access mode) closes
  // itself first.
  const closeRef = useLatest(closePicker);
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      const target = event.target instanceof Element ? event.target : null;
      if (target?.closest('[role="menu"]') && !dialogRef.current?.contains(target)) return;
      event.preventDefault();
      event.stopPropagation();
      closeRef.current();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [open, closeRef]);
  // The backend descriptor is the authority. A provider is a chat runtime when
  // it advertises messages or a model catalog; adding a new adapter must not
  // require another frontend provider-name allowlist.
  const chatAdapters = adapters.filter(adapter => adapter.capabilities.includes("messages") || adapter.models.length > 0);
  // What the picker treats as selected: the optimistic pick while the host
  // switches, else the host's own model.
  const activeHarness = pending?.harness ?? harness;
  const activeModel = pending ? pending.model : model;
  const activeAccount = pending ? pending.account : currentAccount;
  const activeAccountLabel = accountLabel(accounts, activeHarness, activeAccount);
  const current = chatAdapters.find(adapter => adapter.id === activeHarness);
  const currentModel = current?.models.find(option => option.id === (activeModel ?? current.defaultModel));
  // A label remembered from an earlier catalog read names the model before
  // discovery lands; the live catalog always wins once it has the entry.
  const known = knownLabel ? cleanModelLabel(knownLabel) : null;
  const modelLabel = cleanModelLabel(currentModel?.label ?? known ?? activeModel ?? "Default");
  // Some catalogs bake the provider into the model label ("OpenCode Go ·
  // MiMo V2.5"), and prefixing the harness again read as a stutter:
  // "OpenCode · OpenCode Go · MiMo V2.5". When the model label already opens
  // with the harness label, it carries the whole pill by itself.
  const baseHarnessName = harnessLabel(activeHarness);
  const harnessName = activeAccountLabel ? `${baseHarnessName} · ${activeAccountLabel}` : baseHarnessName;
  const modelWithHarness = modelLabel.toLowerCase().startsWith(baseHarnessName.toLowerCase())
    ? modelLabel
    : `${baseHarnessName} · ${modelLabel}`;
  // The account leads, so a narrow pill truncates the model rather than
  // hiding which subscription the chat bills.
  const compactLabel = activeAccountLabel ? `${activeAccountLabel} · ${modelWithHarness}` : modelWithHarness;
  const effortLevels = effortLevelsFrom(currentModel?.supportedEffortLevels?.filter(value => !allowedEfforts || allowedEfforts.includes(value)));
  // A surface that wires neither the value nor a setter does not do effort at
  // all; one that wires either shows the current level, editable or not.
  const effortSurface = effort != null || !!onEffortChange;
  const showEffort = effortLevels.length > 0 && effortSurface;
  const [favorites, toggleFavorite] = useModelFavorites();
  // While the host switches to the picked model the ladder is the new model's;
  // the session's level carries over only if that ladder has it, which is the
  // backend's rule too. A level chosen here wins over both until confirmed.
  const effortValue = queued?.value ?? (pending ? (effortLevels.some(level => level.value === effort) ? effort : null) : effort);
  const changeEffort = (value: string) => {
    if (!onEffortChange) return;
    if (disabled) { setQueued({ value, sent: false }); return; }
    setInFlight(true);
    setQueued({ value, sent: true });
    onEffortChange(value);
  };
  // The picker's own request is outstanding and the host is busy with it.
  // Rows ignore a second pick meanwhile — it would race the first on the
  // session lock — and the effort control stays live so both can be set in
  // one open. Only a disable from elsewhere makes the control inert.
  const switchInFlight = !!disabled && inFlight;
  const effortDisabled = !!disabled && !inFlight;
  const effortProps = { levels: effortLevels, value: effortValue, onChange: onEffortChange ? changeEffort : undefined, disabled: effortDisabled };
  // Faster service tiers the selected model lists. Only a surface that can
  // change the speed shows them; a model without any shows nothing.
  const speedTiers = onSpeedChange ? currentModel?.speedTiers ?? [] : [];
  const speedControl = speedTiers.length > 0
    ? <SpeedControl tiers={speedTiers} value={pending ? null : speed} onChange={onSpeedChange} disabled={!!disabled} />
    : null;
  const pickModel = (nextHarness: Harness, nextModel: string | null, nextAccount: string | null) => {
    if (switchInFlight) return;
    setInFlight(true);
    if (effortSurface) setPending({ harness: nextHarness, model: nextModel, account: nextAccount });
    onChange(nextHarness, nextModel, nextAccount);
    // A surface that does effort keeps the popover open so thinking can be set
    // for the new model in the same open; a plain picker closes on pick.
    if (!effortSurface) closePicker();
  };
  const q = query.trim().toLowerCase();
  const matchesQuery = (label: string) => !q || cleanModelLabel(label).toLowerCase().includes(q);
  // Nothing names this model yet: the host is still resolving the chat's
  // defaults and no catalog entry or remembered label exists. Say so, after a
  // short grace period so a fast local read never flashes a spinner.
  const awaitingModel = !!loading && !currentModel && !known;
  const [showAwaiting, setShowAwaiting] = useState(false);
  useResetOnChange([awaitingModel], () => { if (!awaitingModel) setShowAwaiting(false); });
  useEffect(() => {
    if (!awaitingModel) return;
    const timer = setTimeout(() => setShowAwaiting(true), MODEL_LOADING_DELAY_MS);
    return () => clearTimeout(timer);
  }, [awaitingModel]);

  // One list, every harness in it. The user's own login first, then one group
  // per extra account on the same harness, sharing its catalog. The rail
  // beside it jumps between harnesses rather than hiding the others, so a
  // search still finds a model wherever it lives.
  const groups = chatAdapters
    .flatMap(adapter => [
      { adapter, account: null as string | null, label: adapter.label },
      ...accounts.filter(choice => choice.harness === adapter.id).map(choice => ({ adapter, account: choice.id as string | null, label: `${adapter.label} · ${choice.label}` })),
    ])
    .map(({ adapter, account: groupAccount, label }) => {
      const catalog: ModelOption[] = adapter.models.length ? adapter.models : [{ id: "", label: "Default", tier: "fast", defaultForTier: true }];
      const groupMatches = matchesQuery(label);
      return { adapter, groupAccount, label, models: catalog.filter(option => groupMatches || matchesQuery(option.label)) };
    })
    .filter(group => group.models.length > 0);
  // Starred models, resolved against the live catalog. A favorite whose model
  // or account is gone is skipped rather than drawn as a dead row.
  const favoriteRows = favorites.flatMap(favorite => {
    const adapter = chatAdapters.find(item => item.id === favorite.harness);
    const option = adapter?.models.find(item => item.id === favorite.model);
    if (!adapter || !option) return [];
    const choice = favorite.account ? accounts.find(item => item.harness === adapter.id && item.id === favorite.account) : null;
    if (favorite.account && !choice) return [];
    const label = choice ? `${adapter.label} · ${choice.label}` : adapter.label;
    if (!matchesQuery(label) && !matchesQuery(option.label)) return [];
    return [{ adapter, groupAccount: favorite.account, option, accountName: choice?.label ?? null }];
  });
  const showFavorites = !showAwaiting && favoriteRows.length > 0;

  // The rail: favorites, then one mark per harness. Which one is lit follows
  // the list's scroll; clicking one scrolls the list to it.
  const [section, setSection] = useState<string | null>(null);
  const railSection = section ?? activeHarness;
  const railItems = [
    { section: FAVORITES_SECTION, label: "Favorites", title: "Favorites", dim: false, icon: <Star size={15} aria-hidden="true" className={cn("text-muted-foreground", favoriteRows.length > 0 && "fill-current")} /> },
    ...chatAdapters.map(adapter => ({
      section: adapter.id,
      label: adapter.label,
      title: adapter.available ? adapter.label : `${adapter.label} · unavailable`,
      // A search that finds nothing for a harness dims its mark.
      dim: !!q && !groups.some(group => group.adapter.id === adapter.id),
      icon: <HarnessMark harness={adapter.id} size={16} className={adapter.available ? undefined : "opacity-40"} />,
    })),
  ];
  const railTabStop = railItems.some(item => item.section === railSection) ? railSection : railItems[0]?.section;
  const listRef = useRef<HTMLDivElement>(null);
  // The scrollTop the picker itself last set. The scroll event that follows is
  // ours, not the user's, and must not move the rail off what was clicked
  // (a short last section can never scroll to the top of the list).
  const ownScroll = useRef<number | null>(null);
  const scrollListTo = (target: string, account?: string | null) => {
    const list = listRef.current;
    if (!list) return;
    const sections = [...list.querySelectorAll<HTMLElement>("[data-section]")];
    const element = sections.find(item => item.dataset.section === target && (account === undefined || item.dataset.account === (account ?? "")))
      ?? sections.find(item => item.dataset.section === target);
    if (!element) return;
    const origin = sections[0]?.offsetTop ?? 0;
    let top = element.offsetTop - origin;
    // Opening on a long catalog: keep the chat's own model in view too.
    const row = account !== undefined ? element.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')?.parentElement : null;
    if (row && row.offsetTop + row.offsetHeight > top + list.clientHeight) top = row.offsetTop + row.offsetHeight - list.clientHeight + 8;
    list.scrollTop = Math.max(0, top);
    ownScroll.current = list.scrollTop;
  };
  const [jump, setJump] = useState<{ section: string; seq: number } | null>(null);
  const jumpTo = (target: string) => {
    setSection(target);
    // Scroll after the render: the favorites hint appears or goes with the
    // selection, and offsets read before it would be off by its height.
    setJump(previous => ({ section: target, seq: (previous?.seq ?? 0) + 1 }));
  };
  useLayoutEffect(() => {
    if (jump) scrollListTo(jump.section);
    // scrollListTo reads only refs and the DOM.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jump]);
  // Opening lands on the chat's own harness and account, with focus in the
  // search box so typing filters at once.
  useResetOnChange([open], () => { if (!open) { setSection(null); setJump(null); } });
  useLayoutEffect(() => {
    if (!open) return;
    scrollListTo(activeHarness, activeAccount);
    searchRef.current?.focus({ preventScroll: true });
    // Only on open: later changes to the active model must not yank the list.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);
  // A new search starts at the top of what it found; clearing it goes back to
  // the chat's own harness. Keyed on the query alone, so opening (above) is
  // not undone by it.
  const lastQuery = useRef(q);
  useLayoutEffect(() => {
    if (lastQuery.current === q) return;
    lastQuery.current = q;
    const list = listRef.current;
    if (!list) return;
    // The section to light is read off the list as drawn, so this is a layout effect.
    // oxlint-disable-next-line react/set-state-in-effect -- the highlighted section comes from the rendered list
    if (!q) { setSection(null); scrollListTo(activeHarness, activeAccount); return; }
    list.scrollTop = 0;
    ownScroll.current = 0;
    const first = list.querySelector<HTMLElement>("[data-section]")?.dataset.section;
    if (first) setSection(first);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q]);
  // Lights the rail mark for the section at the top of the list.
  const syncRail = () => {
    const list = listRef.current;
    if (!list) return;
    const sections = [...list.querySelectorAll<HTMLElement>("[data-section]")];
    if (!sections.length) return;
    const origin = sections[0].offsetTop;
    let current = sections[0];
    if (list.scrollTop > 0 && list.scrollTop + list.clientHeight >= list.scrollHeight - 2) current = sections[sections.length - 1];
    else for (const item of sections) if (item.offsetTop - origin <= list.scrollTop + 12) current = item;
    const next = current.dataset.section;
    if (next && next !== railSection) setSection(next);
  };
  const onListScroll = () => {
    const list = listRef.current;
    if (!list) return;
    if (ownScroll.current !== null && Math.abs(list.scrollTop - ownScroll.current) < 2) return;
    ownScroll.current = null;
    syncRail();
  };
  // Starring the first model (or un-starring the last) adds or removes the
  // favorites section above whatever is in view; the rail follows.
  const lastShowFavorites = useRef(showFavorites);
  useLayoutEffect(() => {
    if (lastShowFavorites.current === showFavorites) return;
    lastShowFavorites.current = showFavorites;
    if (open) syncRail();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showFavorites]);

  // Keyboard. Tab and Shift+Tab move between the columns (rail, models,
  // thinking, footer) and wrap inside the popover; arrows move within one;
  // Enter picks; Escape (above) closes and hands focus back to the composer.
  // Rows and rail marks carry a roving tabindex, so Tab skips over them.
  const onPickerKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const dialog = event.currentTarget;
    const target = event.target as HTMLElement;
    // Keys typed in a menu the footer portalled out (access mode) are its own.
    if (!dialog.contains(target) || event.nativeEvent.isComposing) return;
    if (event.key === "Tab") {
      event.preventDefault();
      moveAcrossColumns(dialog, target, event.shiftKey ? -1 : 1);
      return;
    }
    const zone = target.closest<HTMLElement>("[data-picker-zone]")?.dataset.pickerZone;
    if (zone === "rail") {
      const items = [...dialog.querySelectorAll<HTMLButtonElement>("[data-rail]")];
      const index = items.indexOf(target as HTMLButtonElement);
      if (index < 0) return;
      const next = event.key === "ArrowRight" || event.key === "ArrowDown" ? (index + 1) % items.length
        : event.key === "ArrowLeft" || event.key === "ArrowUp" ? (index - 1 + items.length) % items.length
        : event.key === "Home" ? 0
        : event.key === "End" ? items.length - 1
        : null;
      if (next !== null) {
        event.preventDefault();
        items[next].focus();
        jumpTo(items[next].dataset.rail!);
        return;
      }
      if (event.key === "Enter") {
        // Into the harness's models, ready to pick.
        event.preventDefault();
        const picked = target.dataset.rail!;
        jumpTo(picked);
        const row = [...dialog.querySelectorAll<HTMLButtonElement>("[data-model-row]")].find(item => !item.disabled && item.closest<HTMLElement>("[data-section]")?.dataset.section === picked);
        (row ?? searchRef.current)?.focus();
        return;
      }
    }
    if (zone === "models") {
      const rows = [...dialog.querySelectorAll<HTMLButtonElement>("[data-model-row]")].filter(row => !row.disabled);
      if (target === searchRef.current) {
        if (event.key === "ArrowDown") {
          event.preventDefault();
          (rows.find(row => row.getAttribute("aria-selected") === "true" && !row.dataset.favoriteRow) ?? rows[0])?.focus();
        } else if (event.key === "Enter") {
          // Never let Enter reach the composer's form: it would send the draft.
          event.preventDefault();
          if (q) rows[0]?.click();
        }
        return;
      }
      const index = rows.indexOf(target as HTMLButtonElement);
      const next = index < 0 ? undefined
        : event.key === "ArrowDown" ? rows[Math.min(rows.length - 1, index + 1)]
        : event.key === "ArrowUp" ? index === 0 ? searchRef.current : rows[index - 1]
        : event.key === "Home" ? rows[0]
        : event.key === "End" ? rows[rows.length - 1]
        : undefined;
      if (next) { event.preventDefault(); next.focus(); return; }
      if (index >= 0 && event.key === "Enter") { event.preventDefault(); target.click(); return; }
    }
    if (zone === "thinking" && event.key === "Enter") {
      // "Use": the level is already applied as it moves; Enter keeps it and
      // closes. A segment that was only focused is chosen first.
      event.preventDefault();
      const level = target.getAttribute("role") === "radio" && target.getAttribute("aria-checked") !== "true" ? target.dataset.effort : undefined;
      if (level && !effortDisabled) changeEffort(level);
      closePicker();
    }
  };

  // Picking restarts the provider session; said once, as a caption under the
  // thinking row, or under the list on a surface without one.
  const switchNote = <p className="shrink-0 px-3.5 pb-3 pt-1 text-caption leading-4 text-faint">Switching restarts the provider session. History stays.</p>;

  const renderRow = (adapter: AdapterDescriptor, groupAccount: string | null, option: ModelOption, extra: { favoritesRow?: boolean; accountName?: string | null } = {}) => {
    const selected = adapter.id === activeHarness && groupAccount === activeAccount && (option.id ? option.id === activeModel : !activeModel);
    const selectable = adapter.available && option.available !== false && option.compatible !== false;
    const label = cleanModelLabel(option.label);
    // The fallback "Default" row has no catalog id to remember, so no star.
    const favorite: ModelFavorite | null = option.id ? { harness: adapter.id, model: option.id, account: groupAccount } : null;
    const starred = !!favorite && isModelFavorite(favorites, favorite);
    return <div key={`${extra.favoritesRow ? "favorite" : "row"}:${adapter.id}:${groupAccount ?? ""}:${option.id || "default"}`} className="group/row relative">
      <RowButton
        role="option" aria-selected={selected} disabled={!selectable} tabIndex={-1}
        data-model-row="" data-favorite-row={extra.favoritesRow ? "" : undefined}
        onClick={() => pickModel(adapter.id as Harness, option.id || null, groupAccount)}
        tone="selection"
      ><span className={cn("flex min-w-0 flex-1 items-center gap-2.5", favorite && "pr-7", selected && "font-medium")}>
        {/* A fixed gutter, so the check never shifts the labels. */}
        <span aria-hidden="true" className="grid size-4 shrink-0 place-items-center">{selected && <Check size={13} strokeWidth={2.2} />}</span>
        {extra.favoritesRow && <HarnessMark harness={adapter.id} size={13} className={adapter.available ? undefined : "opacity-40"} />}
        <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">{label}</span>
        {extra.accountName && <span className="min-w-0 shrink truncate text-caption font-normal text-faint">{extra.accountName}</span>}
        {option.lifecycle === "preview" && <span className="shrink-0 rounded-full border border-border px-1.5 py-0.5 font-mono text-caption font-medium text-muted-foreground">preview</span>}
      </span></RowButton>
      {/* The star waits for the row's hover unless it is already set. */}
      {favorite && <span className={cn("absolute right-1.5 top-1/2 -translate-y-1/2", !starred && "opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100")}><Button
        variant="text" size="icon-2xs" tabIndex={-1}
        aria-label={starred ? `Remove ${label} from favorites` : `Add ${label} to favorites`}
        tooltip={starred ? "Remove from favorites" : "Add to favorites"}
        onClick={() => toggleFavorite(favorite)}
        aria-pressed={starred}
      >
        <Star aria-hidden="true" className={starred ? "fill-current" : undefined} />
      </Button></span>}
    </div>;
  };

  return <div ref={wrapper} className={cn("relative min-w-0", maxWidthClassName)}>
    <Button variant={compact ? "quiet" : "ghost"} size={compact ? "sm" : "xs"} className="max-w-full" disabled={disabled} onClick={() => setOpen(value => !value)} tooltip={open ? undefined : showAwaiting ? "Loading models" : disabled ? disabledReason ?? "Model selection is temporarily unavailable" : compactLabel} aria-expanded={open} aria-haspopup="dialog" aria-busy={showAwaiting || undefined} aria-label={showAwaiting ? `${roleLabel} model: loading` : `${roleLabel} model: ${harnessName} ${modelLabel}`}>
      {showAwaiting
        ? <><Skeleton className="size-3.5 shrink-0 rounded-full" /><Skeleton className="h-2.5 w-20" /></>
        : <><HarnessMark harness={activeHarness} size={14} /><span className="whitespace-nowrap overflow-hidden text-ellipsis">{compactLabel}</span></>}
      <ChevronDown size={compact ? 14 : 12} className={cn("shrink-0 text-muted-foreground transition-transform", open && "rotate-180", disabled && "opacity-40")} aria-hidden="true" />
    </Button>
    {open && <>
      <div className="fixed inset-0 z-30" onClick={closePicker} />
      <div
        ref={dialogRef}
        role="dialog" aria-label="Choose a model"
        className={cn("u-glass-popover absolute left-0 z-40 flex max-h-[520px] w-[380px] max-w-[calc(100vw-2rem)] flex-col overflow-y-auto overflow-x-hidden overscroll-contain rounded-xl border border-border bg-popover", side === "down" ? "top-full mt-2" : "bottom-full mb-2")}
        style={fit ? { maxHeight: fit.maxHeight } : undefined}
        onKeyDown={onPickerKeyDown}
      >
        {/* One column, top to bottom: search, the harness strip, the list,
            thinking, then speed and access. The strip comes first in the
            document so Tab reaches it before the search box; `order` draws it
            under the search. */}
        <div role="toolbar" aria-label="Harnesses" aria-orientation="horizontal" data-picker-zone="rail" className="order-2 flex h-11 shrink-0 items-center gap-1.5 overflow-x-auto border-b border-border px-3">
          {railItems.map(item => {
            const lit = item.section === railSection;
            return <Tip key={item.section} label={item.title} side="bottom"><Button
              variant="quiet" size="icon-sm" data-rail={item.section}
              tabIndex={item.section === railTabStop ? 0 : -1}
              aria-label={item.label} aria-current={lit || undefined}
              onClick={() => jumpTo(item.section)}
            >
              {lit && <span aria-hidden="true" className="absolute -bottom-1.5 h-0.5 w-4 rounded-full bg-foreground" />}
              <span className={cn("grid place-items-center", item.dim && "opacity-40")}>{item.icon}</span>
            </Button></Tip>;
          })}
        </div>
        <div data-picker-zone="models" className="contents">
          <div className="order-1 flex h-11 shrink-0 items-center gap-2.5 border-b border-border pl-3.5 pr-2.5">
            <Search size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <Input
              unstyled
              className="min-w-0 flex-1"
              ref={searchRef}
              type="text"
              value={query}
              onChange={event => setQuery(event.target.value)}
              placeholder="Search models…"
              aria-label="Search models"
            />
            {onRefresh && <Button variant="quiet" size="icon-xs" disabled={refreshing} loading={refreshing} aria-label="Refresh model catalogues" tooltip="Refresh model catalogues" onClick={() => {
              setRefreshing(true);
              setRefreshError(null);
              Promise.resolve().then(onRefresh).catch(() => setRefreshError("Could not refresh models. Try again.")).finally(() => setRefreshing(false));
            }}>
              <RefreshCw aria-hidden="true" />
            </Button>}
          </div>
          {refreshError && <p role="alert" className="order-2 px-3 py-2 text-xs text-destructive">{refreshError}</p>}
          <div ref={listRef} onScroll={onListScroll} role="listbox" aria-label={`${roleLabel} models`} aria-busy={switchInFlight || showAwaiting || undefined} className={cn("relative order-3 min-h-36 flex-1 overflow-y-auto px-2 pb-2", switchInFlight && "cursor-progress")}>
                {showAwaiting
                  ? <SkeletonRows label="Loading models" rows={4} meta={false} framed={false} density="menu" />
                  : <>
                    {showFavorites && <div role="group" aria-label="Favorites" data-section={FAVORITES_SECTION}>
                      <div className={GROUP_HEADER}>
                        <span aria-hidden="true" className="grid size-4 shrink-0 place-items-center"><Star size={12} className="fill-current text-muted-foreground opacity-70" /></span>
                        <span className="text-caption font-medium text-muted-foreground">Favorites</span>
                      </div>
                      {favoriteRows.map(row => renderRow(row.adapter, row.groupAccount, row.option, { favoritesRow: true, accountName: row.accountName }))}
                    </div>}
                    {!showFavorites && railSection === FAVORITES_SECTION && <p data-section={FAVORITES_SECTION} className="px-2 pb-2 pt-2 text-meta leading-5 text-muted-foreground">
                      {q ? "No favorites match this search." : "No favorites yet. Star a model to keep it here."}
                    </p>}
                    {groups.map(({ adapter, groupAccount, label, models }, index) => (
                      <div key={`${adapter.id}:${groupAccount ?? ""}`} role="group" aria-label={label} data-section={adapter.id} data-account={groupAccount ?? ""}>
                        {/* Group header: a small label, quiet enough to read as a
                            section marker rather than another row competing with
                            the models beneath it. An unavailable harness is
                            dimmed and flagged, its rows shown but greyed. */}
                        <div className={cn(GROUP_HEADER, (index > 0 || showFavorites) && "mt-1")}>
                          <span aria-hidden="true" className="grid size-4 shrink-0 place-items-center"><HarnessMark harness={adapter.id} size={12} className={cn("opacity-70", !adapter.available && "opacity-30")} /></span>
                          <span className="min-w-0 flex-1 truncate text-caption font-medium text-muted-foreground">{label}</span>
                          {adapter.capabilities.includes("reasoning") && <span className="shrink-0 rounded-full border border-border px-1.5 py-0.5 font-mono text-caption font-medium text-muted-foreground">thinking</span>}
                          {!adapter.available && <span className="shrink-0 text-caption text-muted-foreground">unavailable</span>}
                        </div>
                        {models.map(option => renderRow(adapter, groupAccount, option))}
                      </div>
                    ))}
                  </>}
          </div>
          {!effortSurface && !tight && <div className="order-4">{switchNote}</div>}
        </div>
        {/* Thinking: a label, what the level means, and one segmented row
            under the list, so a model and its effort are set in one open. */}
        {effortSurface && <div data-picker-zone="thinking" className="order-4 flex shrink-0 flex-col border-t border-border">
          <div className="flex flex-col gap-1.5 px-3 pb-1 pt-2.5">
            <div className="flex items-baseline gap-3 px-0.5">
              <span className="shrink-0 text-ui font-medium text-foreground">Thinking</span>
              {/* What the level means; the row alone only says which it is. */}
              {!showAwaiting && showEffort && <span className="ml-auto min-w-0 truncate text-caption text-muted-foreground">{effortIndex(effortLevels, effortValue) >= 0 ? effortMeaning(effortValue!) : "The provider's own default."}</span>}
            </div>
            {showAwaiting ? null : showEffort
              ? <div data-testid="effort-control"><EffortControl {...effortProps} /></div>
              : <p className="px-0.5 text-meta leading-5 text-muted-foreground">No thinking control for this model.</p>}
          </div>
          {tight ? <div className="h-2" /> : switchNote}
        </div>}
        {/* Footer: speed where the model has a faster tier, and the access
            mode. They wrap rather than overlap when both are wide. */}
        {(speedControl || accessControl) && <div data-picker-zone="footer" className="order-5 flex min-h-11 shrink-0 flex-wrap items-center gap-3 border-t border-border px-3 py-2">
          {speedControl}
          {accessControl && <div className="ml-auto flex shrink-0 items-center">{accessControl}</div>}
        </div>}
      </div>
    </>}
  </div>;
}

/** The rail and list section for starred models. Not a harness id: those are
 *  plain words, and this one cannot collide with any of them. */
const FAVORITES_SECTION = "@favorites";

/** A section label that stays at the top of the list while its models scroll
 *  under it, with its badges kept to the right and never squeezed together. */
const GROUP_HEADER = "sticky top-0 z-10 flex h-8 items-center gap-2.5 bg-popover pl-1.5 pr-2";

/** Moves focus to the next (or previous) tab stop inside the picker, wrapping
 *  at either end. Rows, rail marks and slider ticks carry tabindex -1, so the
 *  stops are one per column plus the footer's controls. From an element that
 *  is not itself a stop (a row reached with the arrows), it continues from
 *  that element's place in the document. */
function moveAcrossColumns(dialog: HTMLElement, from: HTMLElement, step: 1 | -1) {
  const stops = [...dialog.querySelectorAll<HTMLElement>("button, input, [tabindex]")]
    .filter(element => element.tabIndex >= 0 && !(element as HTMLButtonElement).disabled && !element.closest('[aria-hidden="true"]'));
  if (!stops.length) return;
  let index = stops.indexOf(from);
  if (index < 0) {
    const following = stops.findIndex(stop => !!(from.compareDocumentPosition(stop) & Node.DOCUMENT_POSITION_FOLLOWING));
    const boundary = following < 0 ? stops.length : following;
    index = step > 0 ? boundary - 1 : boundary;
  }
  stops[(index + step + stops.length) % stops.length].focus();
}
