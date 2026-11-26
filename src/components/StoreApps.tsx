// The Store's Apps tab: every app Bridge has, where it shows up, whether it
// is on, and a pin.
//
// Core owns the list and the state (`apps/list_apps`). Turning an app off
// removes its launcher entry, its pin, its badge, and its agent tools from
// new chats. The Store is where an app is turned back on, so it never lists
// itself.

import { useState } from "react";
import { AppWindow, Pin, PinOff, Power, PowerOff, Trash2 } from "lucide-react";
import { BUILTIN_APPS, CARD_APPS, SLOT_LABELS, personalKey, type AppId, type AppKey } from "../apps/registry";
import { setPinned } from "../apps/pins";
import { applyAnswer, loadApps, useApps, usePersonalApps } from "../apps/store";
import { connectionLabel } from "./personalAppShared";
import { bridgeApi } from "../api";
import { errorMessage } from "../errors";
import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { AppSummary, PersonalAppEntry } from "../protocol/generated/protocol";

type Row = {
  id: string;
  name: string;
  description: string;
  icon: (typeof BUILTIN_APPS)[number]["icon"];
  /** The app opens a screen; card-only apps have none. */
  opens: AppId | null;
  slots: string[];
  summary: AppSummary | undefined;
};

function rows(apps: readonly AppSummary[] | null): Row[] {
  const byId = new Map((apps ?? []).map(app => [app.manifest.id, app]));
  const screens: Row[] = BUILTIN_APPS.filter(app => app.id !== "store").map(app => ({
    id: app.id,
    name: byId.get(app.id)?.manifest.name ?? app.name,
    description: byId.get(app.id)?.manifest.description ?? app.description,
    icon: app.icon,
    opens: app.id,
    slots: app.slots.map(slot => SLOT_LABELS[slot]),
    summary: byId.get(app.id),
  }));
  const cards: Row[] = CARD_APPS.map(app => ({
    id: app.id,
    name: byId.get(app.id)?.manifest.name ?? app.name,
    description: byId.get(app.id)?.manifest.description ?? app.description,
    icon: app.icon,
    opens: null,
    slots: ["Chat card"],
    summary: byId.get(app.id),
  }));
  return [...screens, ...cards];
}

export function StoreApps({ onOpenApp }: { onOpenApp?: (key: AppKey) => void }) {
  const apps = useApps();
  const personal = usePersonalApps();
  const [status, setStatus] = useState<string | null>(null);

  const pin = async (row: Row, pinned: boolean) => {
    if (!row.opens) return;
    const result = await setPinned(row.opens, !pinned);
    setStatus(!result.ok ? result.message : pinned ? `Unpinned ${row.name}.` : `Pinned ${row.name} to the sidebar.`);
  };

  const power = async (row: Row, enabled: boolean) => {
    try {
      applyAnswer(await bridgeApi.setAppEnabled(row.id, !enabled));
      setStatus(enabled ? `Turned ${row.name} off. New chats no longer get its tools.` : `Turned ${row.name} on.`);
    } catch (error) {
      setStatus(errorMessage(error));
    }
  };

  return (
    <div className="h-full min-h-0 overflow-y-auto">
    <div className={SCREEN_CONTENT}>
      <ScreenHeading title="Apps" />
      <p role="status" className={status ? "mb-3 text-meta text-foreground" : "sr-only"}>{status}</p>
      {/* A container, so the grid counts its columns from this column's own
          width: the sidebar and the zoom level narrow it without touching the
          window, and viewport breakpoints kept two columns in half the room. */}
      <div className="@container"><ul aria-label="Installed apps" className="grid gap-2.5 @min-[560px]:grid-cols-2 @min-[880px]:grid-cols-3">
        {rows(apps).map(row => {
          const Icon = row.icon;
          const enabled = row.summary?.enabled ?? true;
          const pinned = row.summary?.pinned !== null && row.summary?.pinned !== undefined;
          const canDisable = row.summary?.canDisable ?? true;
          return (
            <li key={row.id} data-app={row.id} data-enabled={enabled ? "true" : "false"} className="flex flex-col gap-3 rounded-xl border border-border bg-card p-3.5">
              <div className="flex items-start gap-3">
                <span className={cn("flex size-10 shrink-0 items-center justify-center rounded-xl border border-border-card bg-background shadow-control", !enabled && "opacity-50")}><Icon size={18} strokeWidth={1.5} aria-hidden="true" /></span>
                <div className="min-w-0 flex-1">
                  <h3 className="flex items-center gap-1.5 text-ui font-medium text-foreground">
                    {row.name}
                    {pinned && <Pin size={11} className="fill-current text-muted-foreground" aria-label="Pinned" />}
                    {!enabled && <span className="rounded-md bg-muted px-1.5 py-0.5 text-caption font-normal text-muted-foreground">Off</span>}
                  </h3>
                  <p className="text-meta leading-snug text-muted-foreground">{row.description}</p>
                </div>
              </div>
              <div className="mt-auto flex flex-wrap items-center gap-1">
                {row.slots.map(slot => <span key={slot} className="whitespace-nowrap rounded-md bg-muted px-1.5 py-0.5 text-caption text-muted-foreground">{slot}</span>)}
                <span className="ml-auto flex flex-wrap items-center justify-end gap-1">
                  {canDisable && (
                    <Button size="sm" variant="ghost" onClick={() => void power(row, enabled)} aria-label={`Turn ${row.name} ${enabled ? "off" : "on"}`}>
                      {enabled ? <PowerOff aria-hidden="true" /> : <Power aria-hidden="true" />}
                      {enabled ? "Turn off" : "Turn on"}
                    </Button>
                  )}
                  {row.opens && enabled && (
                    <Button size="sm" variant="ghost" onClick={() => void pin(row, pinned)} aria-label={`${pinned ? "Unpin" : "Pin"} ${row.name}`}>
                      {pinned ? <PinOff aria-hidden="true" /> : <Pin aria-hidden="true" />}
                      {pinned ? "Unpin" : "Pin"}
                    </Button>
                  )}
                  {row.opens && enabled && onOpenApp && <Button size="sm" variant="outline" onClick={() => onOpenApp(row.opens!)} aria-label={`Open ${row.name}`}>Open</Button>}
                </span>
              </div>
            </li>
          );
        })}
      </ul></div>
      <MyApps apps={personal} onOpenApp={onOpenApp} onStatus={setStatus} />
    </div>
    </div>
  );
}

/** The person's own locked apps. Each opens in its own pane, pins like any
 *  app, and can be turned off (its waiting writes are revoked) or deleted. */
function MyApps({ apps, onOpenApp, onStatus }: { apps: readonly PersonalAppEntry[]; onOpenApp?: (key: AppKey) => void; onStatus: (text: string) => void }) {
  const [confirming, setConfirming] = useState<string | null>(null);

  const pin = async (entry: PersonalAppEntry, pinned: boolean) => {
    const result = await setPinned(personalKey(entry.app.id), !pinned);
    onStatus(!result.ok ? result.message : pinned ? `Unpinned ${entry.app.name}.` : `Pinned ${entry.app.name} to the sidebar.`);
  };
  const power = async (entry: PersonalAppEntry) => {
    try {
      applyAnswer(await bridgeApi.setAppEnabled(entry.app.id, !entry.enabled));
      onStatus(entry.enabled ? `Turned ${entry.app.name} off. Writes waiting for approval were cancelled.` : `Turned ${entry.app.name} on.`);
    } catch (error) {
      onStatus(errorMessage(error));
    }
  };
  const remove = async (entry: PersonalAppEntry) => {
    setConfirming(null);
    try {
      await bridgeApi.deletePersonalApp(entry.app.id);
      await loadApps();
      onStatus(`Deleted ${entry.app.name}.`);
    } catch (error) {
      onStatus(errorMessage(error));
    }
  };

  return <section aria-labelledby="my-apps-heading" className="mt-8">
    <h2 id="my-apps-heading" className="mb-1 text-body font-medium text-foreground">My apps</h2>
    {apps.length === 0
      ? <p className="rounded-xl border border-dashed border-border px-3.5 py-3 text-meta text-muted-foreground">No apps yet. Ask a chat to build one for an MCP server you use, then lock it.</p>
      : <div className="@container"><ul aria-label="My apps" className="grid gap-2.5 @min-[560px]:grid-cols-2 @min-[880px]:grid-cols-3">
        {apps.map(entry => {
          const { app, enabled } = entry;
          const pinned = entry.pinned !== null && entry.pinned !== undefined;
          return <li key={app.id} data-personal-app={app.id} data-enabled={enabled ? "true" : "false"} className="flex flex-col gap-3 rounded-xl border border-border bg-card p-3.5">
            <div className="flex items-start gap-3">
              <span className={cn("flex size-10 shrink-0 items-center justify-center rounded-xl border border-border-card bg-background shadow-control", !enabled && "opacity-50")}><AppWindow size={18} strokeWidth={1.5} aria-hidden="true" /></span>
              <div className="min-w-0 flex-1">
                <h3 className="flex items-center gap-1.5 text-ui font-medium text-foreground">
                  {app.name}
                  {pinned && <Pin size={11} className="fill-current text-muted-foreground" aria-label="Pinned" />}
                  {!enabled && <span className="rounded-md bg-muted px-1.5 py-0.5 text-caption font-normal text-muted-foreground">Off</span>}
                </h3>
                <p className="text-meta leading-snug text-muted-foreground">{app.intent}</p>
                <p className="mt-1 truncate text-caption text-muted-foreground">{app.server} · via {connectionLabel(app.harness)} · v{app.version}</p>
              </div>
            </div>
            {confirming === app.id
              ? <div role="group" aria-label={`Delete ${app.name}?`} className="mt-auto flex flex-wrap items-center gap-1.5 text-meta">
                <span className="min-w-0 flex-1 text-foreground">Delete every version of {app.name}?</span>
                <Button size="sm" variant="ghost" onClick={() => setConfirming(null)}>Cancel</Button>
                <Button size="sm" variant="destructive" onClick={() => void remove(entry)}>Delete</Button>
              </div>
              : <div className="mt-auto flex flex-wrap items-center justify-end gap-1">
                <Button size="sm" variant="ghost" onClick={() => setConfirming(app.id)} aria-label={`Delete ${app.name}`}><Trash2 aria-hidden="true" />Delete</Button>
                <Button size="sm" variant="ghost" onClick={() => void power(entry)} aria-label={`Turn ${app.name} ${enabled ? "off" : "on"}`}>
                  {enabled ? <PowerOff aria-hidden="true" /> : <Power aria-hidden="true" />}
                  {enabled ? "Turn off" : "Turn on"}
                </Button>
                {enabled && <Button size="sm" variant="ghost" onClick={() => void pin(entry, pinned)} aria-label={`${pinned ? "Unpin" : "Pin"} ${app.name}`}>
                  {pinned ? <PinOff aria-hidden="true" /> : <Pin aria-hidden="true" />}
                  {pinned ? "Unpin" : "Pin"}
                </Button>}
                {enabled && onOpenApp && <Button size="sm" variant="outline" onClick={() => onOpenApp(personalKey(app.id))} aria-label={`Open ${app.name}`}>Open</Button>}
              </div>}
          </li>;
        })}
      </ul></div>}
  </section>;
}
