// scratch utility for testing PR workflow — not wired into the app

export interface TextStats {
  chars: number;
  words: number;
  lines: number;
  sentences: number;
  averageWordLength: number;
}

export function countWords(text: string): number {
  const trimmed = text.trim();
  if (trimmed.length === 0) return 0;
  return trimmed.split(/\s+/).length;
}

export function countLines(text: string): number {
  if (text.length === 0) return 0;
  return text.split(/\r\n|\r|\n/).length;
}

export function countSentences(text: string): number {
  const matches = text.match(/[^.!?]+[.!?]+/g);
  return matches ? matches.length : text.trim().length > 0 ? 1 : 0;
}

export function averageWordLength(text: string): number {
  const words = text.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return 0;
  const total = words.reduce((sum, word) => sum + word.replace(/[^\w]/g, "").length, 0);
  return total / words.length;
}

export function getTextStats(text: string): TextStats {
  return {
    chars: text.length,
    words: countWords(text),
    lines: countLines(text),
    sentences: countSentences(text),
    averageWordLength: averageWordLength(text),
  };
}

export function truncateWithEllipsis(text: string, maxLength: number): string {
  if (maxLength < 0) throw new Error("maxLength must be non-negative");
  if (text.length <= maxLength) return text;
  if (maxLength <= 1) return text.slice(0, maxLength);
  return `${text.slice(0, maxLength - 1)}…`;
}

export function slugify(text: string): string {
  return text
    .toLowerCase()
    .trim()
    .replace(/[^\w\s-]/g, "")
    .replace(/[\s_]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export function titleCase(text: string): string {
  return text
    .toLowerCase()
    .split(/\s+/)
    .map((word) => (word.length > 0 ? word[0].toUpperCase() + word.slice(1) : word))
    .join(" ");
}
