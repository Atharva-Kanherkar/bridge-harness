import type { AttentionItem } from "./types";

// Two short tones through the Web Audio API, synthesized so no audio file
// ships. A request rises (it wants an answer); a failed or finished turn gets
// one soft tone. Playing is best effort: no device or an autoplay policy that
// refuses is not worth an error.

/** Frequencies in Hz, played one after another. */
export const SOUND_PATTERNS: Record<"request" | "turn", readonly number[]> = {
  request: [660, 880],
  turn: [523],
};
const NOTE_SECONDS = 0.12;
const GAP_SECONDS = 0.04;
const PEAK_GAIN = 0.08;

export function soundFor(kind: AttentionItem["kind"]): readonly number[] {
  return kind === "needsYou" || kind === "escalation" ? SOUND_PATTERNS.request : SOUND_PATTERNS.turn;
}

let context: AudioContext | undefined;

export function playAttentionSound(kind: AttentionItem["kind"], createContext: () => AudioContext = () => new AudioContext()): void {
  try {
    context ??= createContext();
    const audio = context;
    void audio.resume?.().catch(() => undefined);
    let start = audio.currentTime;
    for (const frequency of soundFor(kind)) {
      const oscillator = audio.createOscillator();
      const gain = audio.createGain();
      oscillator.type = "sine";
      oscillator.frequency.value = frequency;
      // A quick fade in and out, so the tone has no click.
      gain.gain.setValueAtTime(0, start);
      gain.gain.linearRampToValueAtTime(PEAK_GAIN, start + 0.01);
      gain.gain.linearRampToValueAtTime(0, start + NOTE_SECONDS);
      oscillator.connect(gain).connect(audio.destination);
      oscillator.start(start);
      oscillator.stop(start + NOTE_SECONDS);
      start += NOTE_SECONDS + GAP_SECONDS;
    }
  } catch {
    context = undefined;
  }
}

/** Tests only: forget the shared context. */
export function resetAttentionSound(): void {
  context = undefined;
}
