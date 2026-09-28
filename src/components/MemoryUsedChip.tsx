import { Pin } from "lucide-react";
import type { MemoryPacketAudit } from "../types";

/**
 * "Memory used (N)" above the composer — audit-backed, never inferred from
 * events, and absent at zero: a count that is always present is a count that
 * stops being read. The chip opens Bridge's Memory screen.
 */
export function MemoryUsedChip({
  audit,
  onOpenMemory,
}: {
  audit: MemoryPacketAudit | null;
  onOpenMemory: () => void;
}) {
  if (!audit || audit.selected.length === 0) return null;
  return <div className="mx-auto mb-2 flex w-full max-w-2xl justify-center px-4 sm:px-6">
    <button
      type="button"
      aria-label={`Open Memory, ${audit.selected.length} memories used in this session`}
      onClick={onOpenMemory}
      className="u-glass-soft inline-flex h-[30px] items-center gap-2 rounded-full px-3.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
    >
      <Pin size={12} aria-hidden="true" />
      Memory used ({audit.selected.length})
    </button>
  </div>;
}
