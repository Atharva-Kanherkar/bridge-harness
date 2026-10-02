import { useCallback, useState } from "react";
import { cn } from "@/lib/utils";
import type { BrowserCloneStatus } from "../types";
import { CloneSurface, type CloneSupervision } from "./CloneSurface";
import { SimpleBrowser, type SimpleBrowserProps } from "./SimpleBrowser";

// The dock's one browser. It is your own browser until the agent asks for a
// throwaway copy of a site (or you open one), then the same pane shows that
// copy with the agent's pointer on it, and goes back to your pages when the
// copy is gone. Both halves stay mounted so neither loses its tabs or its
// clone when the other is showing.

const engaged = (status: BrowserCloneStatus) => status !== "none" && status !== "destroyed";

export function BrowserPane({ agentLabel, onError, onSupervisionChange, ...browser }: SimpleBrowserProps & {
  agentLabel?: string;
  onError: (message: string) => void;
  onSupervisionChange?: (state: CloneSupervision) => void;
}) {
  const [status, setStatus] = useState<BrowserCloneStatus>("none");
  const [starting, setStarting] = useState(false);
  const report = useCallback((state: CloneSupervision) => {
    setStatus(state.status);
    if (engaged(state.status)) setStarting(false);
    onSupervisionChange?.(state);
  }, [onSupervisionChange]);
  const showClone = engaged(status) || starting;
  const visible = browser.visible ?? true;

  return <div className="relative h-full min-w-0">
    <div className={cn("h-full", showClone && "hidden")}>
      <SimpleBrowser {...browser} visible={visible && !showClone} onStartThrowaway={() => setStarting(true)} />
    </div>
    <div className={cn("h-full", !showClone && "hidden")}>
      <CloneSurface visible={visible} sessionId={browser.sessionId} agentLabel={agentLabel} onCancelStart={() => setStarting(false)} onSupervisionChange={report} onError={onError} />
    </div>
  </div>;
}
