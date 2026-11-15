// The Store's Apps tab: every app Bridge has, where it shows up, whether it
// is on, and a pin.
//
// Core owns the list and the state (`apps/list_apps`). Turning an app off
// removes its launcher entry, its pin, its badge, and its agent tools from
// new chats. The Store is where an app is turned back on, so it never lists
// itself.

import { useState } from "react";
import { Pin, PinOff, Power, PowerOff } from "lucide-react";
import { BUILTIN_APPS, CARD_APPS, SLOT_LABELS, type AppId } from "../apps/registry";
import { setPinned } from "../apps/pins";
import { applyAnswer, useApps } from "../apps/store";
import { bridgeApi } from "../api";
import { errorMessage } from "../errors";
import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { chordLabel } from "../keymap";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { AppSummary } from "../protocol/generated/protocol";

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

export function StoreApps({ onOpenApp }: { onOpenApp?: (id: AppId) => void }) {
  const apps = useApps();
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
      <ScreenHeading title="Apps" description={`Everything Bridge does, one app each. Open any of them with ${chordLabel("open-launcher")}, pin up to three to the sidebar, and turn off the ones you do not use.`} />
      <p role="status" className={status ? "mb-3 text-[12.5px] text-foreground" : "sr-only"}>{status}</p>
      <ul aria-label="Installed apps" className="grid gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
        {rows(apps).map(row => {
          const Icon = row.icon;
          const enabled = row.summary?.enabled ?? true;
          const pinned = row.summary?.pinned !== null && row.summary?.pinned !== undefined;
          const canDisable = row.summary?.canDisable ?? true;
          return (
            <li key={row.id} data-app={row.id} data-enabled={enabled ? "true" : "false"} className="flex flex-col gap-3 rounded-xl border border-border bg-card p-3.5">
              <div className="flex items-start gap-3">
                <span className={cn("flex size-10 shrink-0 items-center justify-center rounded-[11px] border border-border-card bg-background shadow-control", !enabled && "opacity-50")}><Icon size={18} strokeWidth={1.5} aria-hidden="true" /></span>
                <div className="min-w-0 flex-1">
                  <h3 className="flex items-center gap-1.5 text-[13.5px] font-medium text-foreground">
                    {row.name}
                    {pinned && <Pin size={11} className="fill-current text-muted-foreground" aria-label="Pinned" />}
                    {!enabled && <span className="rounded-md bg-muted px-1.5 py-0.5 text-[11px] font-normal text-muted-foreground">Off</span>}
                  </h3>
                  <p className="text-[12px] leading-snug text-muted-foreground">{row.description}</p>
                </div>
              </div>
              <div className="mt-auto flex flex-wrap items-center gap-1">
                {row.slots.map(slot => <span key={slot} className="whitespace-nowrap rounded-md bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">{slot}</span>)}
                <span className="ml-auto flex items-center gap-1">
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
      </ul>
    </div>
    </div>
  );
}
