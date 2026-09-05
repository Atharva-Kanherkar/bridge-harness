// Vocabulary for the thinking-effort control, keyed by the wire effort value.
// Every style draws the same levels — only how it says them differs. A wire
// value Bridge has not seen falls back to itself so a new provider level is
// never hidden, only unstyled.

export type EffortLevel = { value: string; label: string };

/** Compact glyphs for the footer's single label. */
const LABELS: Record<string, string> = {
  low: "Low",
  medium: "Med",
  high: "High",
  xhigh: "XHigh",
  max: "Max",
  ultra: "Ultra",
};

/** The adverb the sentence style scrubs through. */
const WORDS: Record<string, string> = {
  low: "lightly",
  medium: "briefly",
  high: "properly",
  xhigh: "deeply",
  max: "exhaustively",
  ultra: "relentlessly",
};

/** One line of meaning per level for the list style. */
const MEANINGS: Record<string, string> = {
  low: "Fastest. Minimal reasoning.",
  medium: "Balanced default.",
  high: "More deliberate, slower.",
  xhigh: "Long reasoning chains.",
  max: "Longest Claude allows.",
  ultra: "Codex ceiling. Slowest.",
};

export function effortLabel(value: string): string {
  return LABELS[value] ?? value.charAt(0).toUpperCase() + value.slice(1);
}

export function effortWord(value: string): string {
  return WORDS[value] ?? value;
}

export function effortMeaning(value: string): string {
  return MEANINGS[value] ?? "";
}

/** De-duplicated levels in the order the catalog reported them. */
export function effortLevelsFrom(values: readonly string[] | undefined): EffortLevel[] {
  return [...new Set(values ?? [])].map(value => ({ value, label: effortLabel(value) }));
}

/** Index of the current value, or -1 when it is not on the ladder. */
export function effortIndex(levels: EffortLevel[], value: string | null | undefined): number {
  return levels.findIndex(level => level.value === value);
}

/** Props every style renders from. The footer decides the form, not the data. */
export type EffortControlProps = {
  levels: EffortLevel[];
  value: string | null | undefined;
  /** Absent means read-only: the control shows the value and stays inert. */
  onChange?: (value: string) => void;
  disabled?: boolean;
  harness: string;
  /** The resolved model's display label, for styles that name it. */
  modelLabel: string;
};
