import type { LimitRecoveryView } from "./protocol/generated/protocol";
import type { AdapterDescriptor } from "./types";

/** What the chat banner says about a usage-limit recovery, or nothing. */
export type LimitRecoveryNotice = {
  tone: "waiting" | "problem";
  text: string;
  /** Resume now and Cancel apply only while Bridge is still waiting. */
  actionable: boolean;
};

/** "Claude · Opus 4.7", from the catalog when it knows the model. */
export function targetLabel(target: LimitRecoveryView["target"], adapters: AdapterDescriptor[]): string {
  const adapter = adapters.find(item => item.id === target.harness);
  const harness = adapter?.label ?? target.harness ?? "the backup";
  const model = target.model ? adapter?.models.find(item => item.id === target.model)?.label ?? target.model : "default model";
  return `${harness} · ${model}`;
}

/** "in 3 h 20 min", "in 4 min", "in under a minute". */
export function untilText(due: Date, now: Date): string {
  const minutes = Math.ceil((due.getTime() - now.getTime()) / 60_000);
  if (minutes <= 1) return "in under a minute";
  if (minutes < 60) return `in ${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `in ${hours} h ${rest} min` : `in ${hours} h`;
}

export function limitRecoveryNotice(view: LimitRecoveryView | null | undefined, adapters: AdapterDescriptor[], now: Date, locale?: string): LimitRecoveryNotice | null {
  if (!view) return null;
  switch (view.state) {
    case "pending": {
      if (view.switchesModel) return { tone: "waiting", text: `Usage limit reached. Switching to ${targetLabel(view.target, adapters)}…`, actionable: true };
      const due = new Date(view.dueAt);
      const clock = due.toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" });
      const why = view.resetAt ? "when the limit resets" : "to check whether the limit has cleared";
      return { tone: "waiting", text: `Usage limit reached. Bridge continues this chat at ${clock} (${untilText(due, now)}), ${why}.`, actionable: true };
    }
    case "acting":
      return { tone: "waiting", text: view.switchesModel ? `Switching to ${targetLabel(view.target, adapters)} and continuing…` : "Continuing after the usage limit…", actionable: false };
    case "failed":
      return { tone: "problem", text: `Bridge could not continue this chat after the usage limit: ${view.outcome ?? "unknown error"}`, actionable: false };
    case "gave_up":
      return { tone: "problem", text: `The usage limit kept coming back (${view.attempts} times), so Bridge stopped retrying. Send a message to continue.`, actionable: false };
    default:
      return null;
  }
}
