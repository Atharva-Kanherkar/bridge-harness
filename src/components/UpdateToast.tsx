import { useState } from "react";
import { Download, LoaderCircle } from "lucide-react";
import { isDevelopmentBuild, UpdateInstallUnavailableError, type UpdateInfo } from "../updater";
import { ToastCard } from "./ToastCard";

/** Last in the top-right stack. It stays until installed or dismissed. */
export function UpdateToast({ update, onInstall, onDismiss }: {
  update: UpdateInfo;
  onInstall: (update: UpdateInfo) => Promise<void>;
  onDismiss: () => void;
}) {
  const [installing, setInstalling] = useState(false);
  const [installError, setInstallError] = useState<string>();
  const developmentBuild = isDevelopmentBuild();
  async function install() {
    setInstallError(undefined);
    setInstalling(true);
    try {
      await onInstall(update);
    } catch (error) {
      setInstalling(false);
      setInstallError(error instanceof UpdateInstallUnavailableError
        ? error.message
        : "Update failed. Check your connection and try again.");
    }
  }
  return <ToastCard
    toastKey={`${update.channel}:${update.version}`}
    durationMs={null}
    onDismiss={onDismiss}
    icon={<Download size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-muted-foreground" />}
    body={<>
      <span className="block text-[13px] font-medium leading-snug text-foreground">Bridge {update.version} is available</span>
      <span className="mt-0.5 block text-[12px] text-muted-foreground">You're on {update.currentVersion}. Update now to restart with the latest build.</span>
    </>}
    footer={<>
      {installError && <span className="mt-1 block text-[12px] text-destructive">{installError}</span>}
      {developmentBuild ? <span className="mt-2 block text-[12px] text-muted-foreground">Install updates from a packaged Bridge app.</span> : <button
        type="button"
        disabled={installing}
        onClick={() => void install()}
        className="mt-2 inline-flex items-center gap-1.5 rounded-md bg-foreground px-2.5 py-1 text-[12px] font-medium text-background transition-opacity hover:opacity-90 disabled:opacity-60"
      >
        {installing && <LoaderCircle size={12} aria-hidden="true" className="animate-spin" />}
        {installing ? "Installing…" : installError ? "Retry" : "Install and restart"}
      </button>}
    </>}
  />;
}
