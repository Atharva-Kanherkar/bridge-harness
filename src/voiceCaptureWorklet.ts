import { ContinuousPcm16Resampler } from "./voiceResampler";

declare const sampleRate: number;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}
declare function registerProcessor(name: string, processor: typeof AudioWorkletProcessor): void;

type ProcessorOptions = {
  targetRate?: number;
  chunkSamples?: number;
};

class BridgeVoiceCaptureProcessor extends AudioWorkletProcessor {
  private readonly resampler: ContinuousPcm16Resampler;
  private accepting = true;

  constructor(options?: { processorOptions?: ProcessorOptions }) {
    super();
    const targetRate = options?.processorOptions?.targetRate ?? 16_000;
    const chunkSamples = options?.processorOptions?.chunkSamples ?? 1_600;
    this.resampler = new ContinuousPcm16Resampler(sampleRate, targetRate, chunkSamples, samples => {
      this.port.postMessage({ type: "chunk", samples: samples.buffer }, [samples.buffer]);
    });
    this.port.onmessage = event => {
      if (event.data?.type !== "flush" || !this.accepting) return;
      this.accepting = false;
      this.resampler.flush();
      this.port.postMessage({ type: "flushed" });
    };
  }

  process(inputs: Float32Array[][]): boolean {
    const channel = inputs[0]?.[0];
    if (this.accepting && channel?.length) this.resampler.push(channel);
    return this.accepting;
  }
}

registerProcessor("bridge-voice-capture", BridgeVoiceCaptureProcessor);
