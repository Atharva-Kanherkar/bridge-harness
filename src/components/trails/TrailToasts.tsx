import { Route } from "lucide-react";
import type { TrailMemberAdded } from "@/trails";
import { ToastCard, ToastOverflow, TOASTS_SHOWN } from "../ToastCard";
import { Button } from "@/components/ui/button";

// "Added to a trail": one card per automatic addition the core announced.
// Undo takes the record off the trail, which the core remembers, so the
// sweep never puts it back.

export const TRAIL_TOAST_TTL_MS = 12_000;

export type TrailToast = { key: string; added: TrailMemberAdded };

export function trailToastKey(added: TrailMemberAdded): string {
  return `${added.trailId}|${added.member.kind}|${added.member.ref}`;
}

export function TrailToasts({ toasts, onOpen, onUndo, onDismiss }: {
  toasts: TrailToast[];
  onOpen: (toast: TrailToast) => void;
  onUndo: (toast: TrailToast) => void;
  onDismiss: (key: string) => void;
}) {
  if (!toasts.length) return null;
  const shown = toasts.slice(-TOASTS_SHOWN);
  return <div className="pointer-events-none flex flex-col items-end gap-2">
    {shown.map(toast => <ToastCard
      key={toast.key}
      toastKey={toast.key}
      durationMs={TRAIL_TOAST_TTL_MS}
      onDismiss={onDismiss}
      icon={<Route size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-muted-foreground" />}
      onOpen={() => onOpen(toast)}
      openLabel={`Added to a trail — open “${toast.added.trailTitle}”`}
      body={<>
        <span className="block text-ui font-medium leading-snug text-foreground">Added to a trail</span>
        <span className="mt-0.5 block text-meta leading-relaxed text-muted-foreground">{toast.added.label} is now on “{toast.added.trailTitle}”.</span>
      </>}
      footer={<div className="mt-2 flex gap-3 text-meta font-medium">
        <Button variant="link" size="inline" onClick={() => onOpen(toast)}>Open trail</Button>
        <Button variant="text" size="xs" onClick={() => onUndo(toast)}>Undo</Button>
      </div>}
    />)}
    <ToastOverflow count={toasts.length - shown.length} />
  </div>;
}
