import { CircleAlert, CircleCheck } from "lucide-react";
import type { ImportJob } from "../protocol/generated/protocol";
import { importNotice } from "../moveToBridge";
import { importSourceLabel } from "../importSources";
import { ToastCard } from "./ToastCard";

// Shown when a background import ends, wherever the person is in Bridge. The
// card opens the Import page with the full summary; the X only dismisses.

export const IMPORT_TOAST_TTL_MS = 15_000;

export function ImportToast({ job, onOpen, onDismiss }: { job: ImportJob; onOpen: () => void; onDismiss: () => void }) {
  const notice = importNotice(job, importSourceLabel(job.tool));
  const Icon = notice.failed ? CircleAlert : CircleCheck;
  return <div className="pointer-events-none flex flex-col items-end gap-2">
    <ToastCard
      toastKey={job.jobId}
      durationMs={IMPORT_TOAST_TTL_MS}
      onDismiss={onDismiss}
      icon={<Icon size={16} aria-hidden="true" className={notice.failed ? "mt-0.5 shrink-0 text-warning" : "mt-0.5 shrink-0 text-success"} />}
      onOpen={onOpen}
      openLabel={`${notice.title}: open the import summary`}
      body={<>
        <span className="block text-[13px] font-medium leading-snug text-foreground">{notice.title}</span>
        <span className="mt-0.5 block text-[12px] text-muted-foreground">{notice.body}</span>
      </>}
    />
  </div>;
}
