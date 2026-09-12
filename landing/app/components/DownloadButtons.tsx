import ActionButton from "./ActionButton";
import { macDownloadPath } from "../content/site";

// Linux has no built app to download yet -- only advertise macOS until a
// Linux build actually ships.
export default function DownloadButtons() {
  return (
    <div className="flex flex-col items-center justify-center gap-4 sm:flex-row">
      <ActionButton href={macDownloadPath} label="Download for macOS" external />
    </div>
  );
}
