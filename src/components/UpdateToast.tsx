import { useState } from "react";
import { Download } from "lucide-react";
import { isDevelopmentBuild, UpdateInstallUnavailableError, type UpdateInfo } from "../updater";
import { friendlyError } from "../errors";
import { ToastCard } from "./ToastCard";
import { Button } from "@/components/ui/button";

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
      const failed = "Update failed. Check your connection and try again.";
      setInstallError(error instanceof UpdateInstallUnavailableError ? friendlyError(error, failed) : failed);
    }
  }
  return <ToastCard
    toastKey={`${update.channel}:${update.version}`}
    durationMs={null}
    onDismiss={onDismiss}
    icon={<Download size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-muted-foreground" />}
    body={<>
      <span className="block text-ui font-medium leading-snug text-foreground">Bridge {update.version} is available</span>
      <span className="mt-0.5 block text-meta text-muted-foreground">You're on {update.currentVersion}. Update now to restart with the latest build.</span>
    </>}
    footer={<>
      {installError && <span className="mt-1 block text-meta text-destructive">{installError}</span>}
      {developmentBuild ? <span className="mt-2 block text-meta text-muted-foreground">Install updates from a packaged Bridge app.</span> : <Button variant="default" size="xs" className="mt-2"
        loading={installing}
        onClick={() => void install()}
      >
        {installError && !installing ? "Retry" : "Install and restart"}
      </Button>}
    </>}
  />;
}
