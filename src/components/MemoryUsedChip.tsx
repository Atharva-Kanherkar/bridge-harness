import type { MemoryPacketAudit } from "../types";

/**
 * "Recalled N memories" at the head of the transcript — audit-backed, never
 * inferred from events, and absent at zero. Memory is injected once, when the
 * session starts, so the note sits where that happened and scrolls away with
 * it rather than floating over the composer for the life of the chat. It opens
 * Bridge's Memory screen.
 */
export function MemoryUsedChip({
  audit,
  onOpenMemory,
}: {
  audit: MemoryPacketAudit | null;
  onOpenMemory: () => void;
}) {
  if (!audit || audit.selected.length === 0) return null;
  const count = audit.selected.length;
  return <div className="flex items-center gap-3 text-caption text-faint">
    <span aria-hidden="true" className="h-px flex-1 bg-border" />
    <button
      type="button"
      aria-label={`Open Memory, ${count} ${count === 1 ? "memory" : "memories"} used in this session`}
      onClick={onOpenMemory}
      className="rounded-md px-1.5 py-0.5 transition-colors hover:text-foreground"
    >
      Recalled {count} {count === 1 ? "memory" : "memories"}
    </button>
    <span aria-hidden="true" className="h-px flex-1 bg-border" />
  </div>;
}
