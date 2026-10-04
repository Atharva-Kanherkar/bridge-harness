const PCM16_MIN = -0x8000;
const PCM16_MAX = 0x7fff;

function pcm16(value: number): number {
  const clamped = Math.max(-1, Math.min(1, value));
  return clamped < 0 ? Math.round(clamped * -PCM16_MIN) : Math.round(clamped * PCM16_MAX);
}

/**
 * Stateful mono resampler used by both the AudioWorklet and its compatibility
 * fallback. Keeping the fractional source position across render quanta is
 * what prevents 44.1/48 kHz buffers from dropping or duplicating their tails.
 */
export class ContinuousPcm16Resampler {
  private readonly ratio: number;
  private input: number[] = [];
  private position = 0;
  private pending: number[] = [];

  constructor(
    inputRate: number,
    targetRate = 16_000,
    private readonly chunkSamples = 1_600,
    private readonly emit: (samples: Int16Array) => void,
  ) {
    if (inputRate <= 0 || targetRate <= 0) throw new Error("Audio sample rates must be positive");
    if (!Number.isSafeInteger(chunkSamples) || chunkSamples <= 0) throw new Error("Audio chunk size must be positive");
    this.ratio = inputRate / targetRate;
  }

  push(samples: Float32Array): void {
    for (const sample of samples) this.input.push(sample);
    this.drain(false);
  }

  flush(): void {
    const before = this.pending.length;
    this.drain(true);
    // A take shorter than one target interval still represents intentional
    // speech input. Preserve its final sample instead of flushing nothing.
    if (this.input.length && this.pending.length === before) this.pending.push(pcm16(this.input.at(-1)!));
    this.input = [];
    this.position = 0;
    this.emitPending(true);
  }

  private drain(final: boolean): void {
    const limit = final ? this.input.length : Math.max(0, this.input.length - 1);
    while (this.position < limit) {
      const left = Math.min(Math.floor(this.position), this.input.length - 1);
      const right = Math.min(left + 1, this.input.length - 1);
      const fraction = this.position - left;
      this.pending.push(pcm16(this.input[left] + (this.input[right] - this.input[left]) * fraction));
      this.position += this.ratio;
      this.emitPending(false);
    }
    if (!final && this.input.length > 1) {
      const consumed = Math.min(Math.floor(this.position), this.input.length - 1);
      if (consumed > 0) {
        this.input = this.input.slice(consumed);
        this.position -= consumed;
      }
    }
  }

  private emitPending(includePartial: boolean): void {
    while (this.pending.length >= this.chunkSamples || (includePartial && this.pending.length)) {
      const length = Math.min(this.chunkSamples, this.pending.length);
      this.emit(Int16Array.from(this.pending.splice(0, length)));
    }
  }
}
