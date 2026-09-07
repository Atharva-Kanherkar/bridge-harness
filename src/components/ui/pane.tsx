import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";

/** Loading, unavailable, and empty states share a readable, actionable layout. */
export function PaneState({ icon: Icon, title, children, action, role }: { icon: LucideIcon; title: string; children?: ReactNode; action?: ReactNode; role?: "status" | "alert" }) {
  return <div role={role} className="flex min-h-48 flex-1 flex-col items-center justify-center gap-3 px-6 py-8 text-center">
    <span className="grid size-10 shrink-0 place-items-center rounded-xl bg-accent text-muted-foreground"><Icon size={20} strokeWidth={1.6} aria-hidden="true" /></span>
    <div className="max-w-sm"><h2 className="text-[14px] font-semibold text-foreground">{title}</h2>{children && <div className="mt-2 text-[13px] leading-relaxed text-muted-foreground">{children}</div>}</div>
    {action && <div className="mt-1 flex flex-wrap items-center justify-center gap-2">{action}</div>}
  </div>;
}
