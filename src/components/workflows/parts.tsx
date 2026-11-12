import type { ReactNode } from "react";
import type { Tone } from "./format";

export const CHIP = "rounded-full border border-border bg-muted px-2 py-0.5 text-caption text-muted-foreground";

const TONE_CHIP: Record<Tone, string> = {
  ok: "border-success/30 bg-success/10 text-success",
  warn: "border-warning/30 bg-warning/10 text-warning",
  bad: "border-destructive/30 bg-destructive/10 text-destructive",
  idle: "border-border bg-muted text-muted-foreground",
};

const TONE_DOT: Record<Tone, string> = {
  ok: "bg-success",
  warn: "bg-warning",
  bad: "bg-destructive",
  idle: "bg-muted-foreground/40",
};

export function StatusChip({ label, tone }: { label: string; tone: Tone }) {
  return <span className={`inline-flex shrink-0 rounded-full border px-1.5 py-0.5 text-caption font-medium uppercase tracking-[0.04em] ${TONE_CHIP[tone]}`}>{label}</span>;
}

export function Dot({ tone }: { tone: Tone }) {
  return <span aria-hidden className={`h-1.5 w-1.5 shrink-0 rounded-full ${TONE_DOT[tone]}`}/>;
}

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return <section className="border-t border-border py-4 first:border-t-0 first:pt-0 last:pb-0">
    <h3 className="mb-2 text-caption font-medium uppercase tracking-[0.06em] text-muted-foreground">{title}</h3>
    {children}
  </section>;
}

export function Card({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`rounded-xl border border-border bg-card p-4 ${className}`}>{children}</div>;
}

export function ErrorNote({ message }: { message: string | null | undefined }) {
  if (!message) return null;
  return <p role="alert" className="mt-3 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-caption text-destructive">{message}</p>;
}

export const errorText = (reason: unknown) => reason instanceof Error ? reason.message : String(reason);
