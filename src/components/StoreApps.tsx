// The Store's Apps tab: every app Bridge has, where it shows up, and a pin.
//
// Built-in apps for now. The same list is what the launcher draws, so the
// Store can never offer an app the launcher cannot open.

import { useState } from "react";
import { Pin, PinOff } from "lucide-react";
import { BUILTIN_APPS, SLOT_LABELS, type AppId } from "../apps/registry";
import { PIN_LIMIT_MESSAGE, togglePin, usePins } from "../apps/pins";
import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { Button } from "@/components/ui/button";

export function StoreApps({ onOpenApp }: { onOpenApp?: (id: AppId) => void }) {
  const pins = usePins();
  const [status, setStatus] = useState<string | null>(null);
  // The Store lists apps; it is not one of the things it lists.
  const apps = BUILTIN_APPS.filter(app => app.id !== "store");

  const toggle = (id: AppId, name: string) => {
    const wasPinned = pins.includes(id);
    const result = togglePin(id);
    setStatus(!result.ok ? PIN_LIMIT_MESSAGE : wasPinned ? `Unpinned ${name}.` : `Pinned ${name} to the sidebar.`);
  };

  return (
    <div className="h-full min-h-0 overflow-y-auto">
    <div className={SCREEN_CONTENT}>
      <ScreenHeading title="Apps" description={`Everything Bridge does, one app each. Open any of them with ⌘K, and pin up to three to the sidebar.`} />
      <p role="status" className={status ? "mb-3 text-[12.5px] text-foreground" : "sr-only"}>{status}</p>
      <ul aria-label="Installed apps" className="grid gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
        {apps.map(app => {
          const Icon = app.icon;
          const pinned = pins.includes(app.id);
          return (
            <li key={app.id} className="flex flex-col gap-3 rounded-xl border border-border bg-card p-3.5">
              <div className="flex items-start gap-3">
                <span className="flex size-10 shrink-0 items-center justify-center rounded-[11px] border border-border-card bg-background shadow-control"><Icon size={18} strokeWidth={1.5} aria-hidden="true" /></span>
                <div className="min-w-0 flex-1">
                  <h3 className="flex items-center gap-1.5 text-[13.5px] font-medium text-foreground">
                    {app.name}
                    {pinned && <Pin size={11} className="fill-current text-muted-foreground" aria-label="Pinned" />}
                  </h3>
                  <p className="text-[12px] leading-snug text-muted-foreground">{app.description}</p>
                </div>
              </div>
              <div className="mt-auto flex items-center gap-1">
                {app.slots.map(slot => <span key={slot} className="whitespace-nowrap rounded-md bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">{SLOT_LABELS[slot]}</span>)}
                <span className="ml-auto flex items-center gap-1">
                  <Button size="sm" variant="ghost" onClick={() => toggle(app.id, app.name)} aria-label={`${pinned ? "Unpin" : "Pin"} ${app.name}`}>
                    {pinned ? <PinOff aria-hidden="true" /> : <Pin aria-hidden="true" />}
                    {pinned ? "Unpin" : "Pin"}
                  </Button>
                  {onOpenApp && <Button size="sm" variant="outline" onClick={() => onOpenApp(app.id)} aria-label={`Open ${app.name}`}>Open</Button>}
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
