import ActionButton from "./ActionButton";
import { linuxDownloadPath, macDownloadPath } from "../content/site";

export default function DownloadButtons() {
  return (
    <div className="flex flex-col items-center justify-center gap-4 sm:flex-row">
      <ActionButton href={macDownloadPath} label="Download for macOS" external />
      <ActionButton href={linuxDownloadPath} label="Download for Linux" external />
    </div>
  );
}
