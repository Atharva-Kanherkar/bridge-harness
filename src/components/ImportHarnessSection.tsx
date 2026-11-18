import { MoveToBridge } from "./MoveToBridge";
import { SettingsPage } from "./settings/kit";

// The Import page: the tools found on this Mac and one Import button each.
// There is no review step and no folder wizard: listing every transcript
// before importing froze the window on a real Mac (85,057 transcripts, 5.1 GB
// of memory). The import runs in the background instead; see `MoveToBridge`.

export function ImportHarnessSection({ onError }: { onError: (message: string) => void }) {
  return <SettingsPage
    title="Import"
    description="Bring your chats, memories and setup over from the tools you already use. Everything stays on this Mac, and secrets are removed before anything is saved."
  >
    <MoveToBridge onError={onError} />
  </SettingsPage>;
}
