import { useEffect, useState } from "react";
import { bridgeApi } from "../api";
import { applyAnswer, usePersonalApps } from "../apps/store";
import { errorMessage } from "../errors";
import type { PersonalAppSurfaceOpen } from "../protocol/generated/protocol";
import { PersonalAppOpenCard } from "./PersonalAppOpenCall";
import { Button } from "./ui/button";

/** Opens and closes per app, run one at a time across every mount. Core
 *  keeps one surface per app and a new open replaces the last, so an open a
 *  pane abandoned must finish (and close itself) before a newer pane's open
 *  starts; otherwise its late reply would replace the newer one. */
const queues = new Map<string, Promise<unknown>>();

export function serialized<T>(appId: string, task: () => Promise<T>): Promise<T> {
  const next = (queues.get(appId) ?? Promise.resolve()).then(task, task);
  const settled = next.then(() => undefined, () => undefined);
  queues.set(appId, settled);
  void settled.then(() => { if (queues.get(appId) === settled) queues.delete(appId); });
  return next;
}

/**
 * One of the person's own apps, opened from the Store or the launcher. The
 * pane holds a Surface open: core makes it on mount (replacing any earlier
 * one for this app) and drops it, with any write waiting in it, on unmount.
 * Nothing here can name a chat, so calls never ride a chat's process.
 */
export function PersonalAppSurface({ appId }: { appId: string }) {
  const apps = usePersonalApps();
  const entry = apps.find(item => item.app.id === appId);
  const enabled = entry?.enabled ?? false;
  const version = entry?.app.version;
  const [open, setOpen] = useState<PersonalAppSurfaceOpen>();
  const [error, setError] = useState("");

  useEffect(() => {
    if (!enabled) return;
    let current = true;
    let made: string | undefined;
    setOpen(undefined);
    setError("");
    void serialized(appId, async () => {
      // A pane that went away before its turn opens nothing.
      if (!current) return;
      try {
        const value = await bridgeApi.openPersonalAppSurface(appId);
        if (!current) {
          await bridgeApi.closePersonalAppSurface(value.openId).catch(() => undefined);
          return;
        }
        made = value.openId;
        setOpen(value);
      } catch (reason) {
        if (current) setError(errorMessage(reason));
      }
    });
    return () => {
      current = false;
      const openId = made;
      if (openId) void serialized(appId, () => bridgeApi.closePersonalAppSurface(openId).catch(() => undefined));
    };
    // A newer locked version opens afresh.
  }, [appId, enabled, version]);

  const turnOn = async () => {
    try {
      applyAnswer(await bridgeApi.setAppEnabled(appId, true));
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  let body: React.ReactNode;
  if (!entry) body = <Notice text="This app was deleted." />;
  else if (!enabled) body = <Notice text={`${entry.app.name} is turned off.`} action={<Button size="sm" variant="outline" onClick={() => void turnOn()}>Turn on</Button>} />;
  else if (error) body = <Notice text={error} />;
  else if (!open) body = <div role="status" className="text-xs text-muted-foreground">Opening {entry.app.name}…</div>;
  else body = <PersonalAppOpenCard key={open.openId} openId={open.openId} contentHash={open.contentHash} readOnly={false} />;

  return <div className="h-full min-h-0 overflow-y-auto" data-personal-app-surface={appId}>
    <div className="mx-auto w-full max-w-3xl px-6 py-6">{body}</div>
  </div>;
}

function Notice({ text, action }: { text: string; action?: React.ReactNode }) {
  return <div role="status" className="flex items-center gap-3 rounded-2xl border border-dashed border-border px-4 py-3 text-sm text-muted-foreground">
    <span className="min-w-0 flex-1">{text}</span>{action}
  </div>;
}
