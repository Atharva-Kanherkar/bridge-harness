import { Pin } from "lucide-react";
import type { MemoryPacketAudit } from "../types";

/**
 * "Memory used (N)" above the composer — audit-backed, never inferred from
 * events, and absent at zero: a count that is always present is a count that
 * stops being read. The disclosure lists each injected item and why.
 */
export function MemoryUsedChip({
  audit,
  open,
  onToggle,
}: {
  audit: MemoryPacketAudit | null;
  open: boolean;
  onToggle: () => void;
}) {
  if (!audit || audit.selected.length === 0) return null;
  return <div className="mx-auto mb-2 flex w-full max-w-2xl flex-col items-center px-4 sm:px-6">
    {open && (
      <div className="u-glass-soft mb-1.5 w-full space-y-1 rounded-2xl px-3.5 py-2.5 text-[12px] text-muted-foreground">
        <p className="text-[11px] font-semibold uppercase tracking-wider">In this session's context</p>
        {audit.selected.map(item => (
          <p key={item.recordId} className="truncate">
            <span className="text-foreground">{item.body}</span> — {item.reason}
          </p>
        ))}
      </div>
    )}
    <button
      type="button"
      aria-expanded={open}
      onClick={onToggle}
      className="u-glass-soft inline-flex h-[30px] items-center gap-2 rounded-full px-3.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
    >
      <Pin size={12} aria-hidden="true" />
      Memory used ({audit.selected.length})
    </button>
  </div>;
}
