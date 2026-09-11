import { useState } from "react";
import { Download, LoaderCircle, X } from "lucide-react";
import type { UpdateInfo } from "../updater";

export function UpdateToast({ update, onInstall, onDismiss }: {
  update: UpdateInfo;
  onInstall: () => Promise<void>;
  onDismiss: () => void;
}) {
  const [installing, setInstalling] = useState(false);
  return <div className="pointer-events-none fixed bottom-3 left-3 z-30 sm:bottom-[18px] sm:left-[18px]">
    <div className="u-glass-popover pointer-events-auto flex w-[min(22rem,calc(100vw-1.5rem))] items-start gap-2.5 rounded-xl border border-border p-3 shadow-2xl animate-page-mount">
      <Download size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <span className="block text-[13px] font-medium leading-snug text-foreground">Bridge {update.version} is available</span>
        <span className="mt-0.5 block text-[12px] text-muted-foreground">You're on {update.currentVersion}. Update now to restart with the latest build.</span>
        <button
          type="button"
          disabled={installing}
          onClick={() => { setInstalling(true); void onInstall(); }}
          className="mt-2 inline-flex items-center gap-1.5 rounded-md bg-foreground px-2.5 py-1 text-[12px] font-medium text-background transition-opacity hover:opacity-90 disabled:opacity-60"
        >
          {installing && <LoaderCircle size={12} aria-hidden="true" className="animate-spin" />}
          {installing ? "Installing…" : "Restart to update"}
        </button>
      </div>
      <button type="button" onClick={onDismiss} aria-label="Dismiss update notification" className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
        <X size={13} aria-hidden="true" />
      </button>
    </div>
  </div>;
}
