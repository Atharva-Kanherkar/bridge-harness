import workletUrl from "./voiceCaptureWorklet.ts?worker&url";
import { ContinuousPcm16Resampler } from "./voiceResampler";

export type VoiceChunk = {
  data: string;
  samplesPerChannel: number;
};

export type VoiceDraftTranscript = {
  finalized: string;
  provisional: string;
};

export function applyVoiceTranscriptDraft(
  current: VoiceDraftTranscript,
  kind: "partial" | "delta" | "final",
  text: string,
): VoiceDraftTranscript {
  if (kind === "partial") return { ...current, provisional: text };
  if (kind === "delta") {
    return { ...current, provisional: current.provisional + text };
  }
  const segment = text.trim();
  return {
    finalized: [current.finalized.trim(), segment].filter(Boolean).join(" "),
    provisional: "",
  };
}

export function renderVoiceDraft(baseDraft: string, transcript: VoiceDraftTranscript): string {
  const spoken = [transcript.finalized.trim(), transcript.provisional.trimStart()]
    .filter(Boolean)
    .join(" ");
  return [baseDraft.trimEnd(), spoken].filter(Boolean).join(" ");
}

export function resampleToPcm16(input: Float32Array, inputRate: number, targetRate = 16_000): Int16Array {
  const output: number[] = [];
  const resampler = new ContinuousPcm16Resampler(inputRate, targetRate, Math.max(1, input.length), samples => output.push(...samples));
  resampler.push(input);
  resampler.flush();
  return Int16Array.from(output);
}

export function pcm16ToBase64(samples: Int16Array): string {
  const bytes = new Uint8Array(samples.length * 2);
  const view = new DataView(bytes.buffer);
  for (let index = 0; index < samples.length; index += 1) {
    view.setInt16(index * 2, samples[index], true);
  }
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(binary);
}

export class VoiceCapture {
  private stopping?: Promise<void>;

  private constructor(
    private readonly stream: MediaStream,
    private readonly context: AudioContext,
    private readonly node: AudioNode,
    private readonly source: MediaStreamAudioSourceNode,
    private readonly mute: GainNode,
    private readonly onEnded: () => void,
    private readonly flush: () => Promise<void>,
    private readonly detach: () => void,
  ) {}

  static async start(onChunk: (chunk: VoiceChunk) => void, onError: (error: Error) => void = () => {}): Promise<VoiceCapture> {
    if (!navigator.mediaDevices?.getUserMedia) throw new Error("Microphone capture is not supported by this app runtime.");
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    });
    let context: AudioContext | undefined;
    let source: MediaStreamAudioSourceNode | undefined;
    let node: AudioNode | undefined;
    let mute: GainNode | undefined;
    let detach = () => undefined;
    const onEnded = () => onError(new Error("The microphone disconnected. Reconnect it and retry dictation."));
    try {
      for (const track of stream.getTracks()) track.addEventListener?.("ended", onEnded);
      context = new AudioContext();
      source = context.createMediaStreamSource(stream);
      mute = context.createGain();
      mute.gain.value = 0;
      const deliver = (pcm: Int16Array) => {
        if (pcm.length) onChunk({ data: pcm16ToBase64(pcm), samplesPerChannel: pcm.length });
      };
      let flush: () => Promise<void>;
      if (context.audioWorklet && typeof AudioWorkletNode !== "undefined") {
        await context.audioWorklet.addModule(workletUrl);
        const worklet = new AudioWorkletNode(context, "bridge-voice-capture", {
          numberOfInputs: 1,
          numberOfOutputs: 1,
          outputChannelCount: [1],
          processorOptions: { targetRate: 16_000, chunkSamples: 1_600 },
        });
        node = worklet;
        let pendingFlush: { resolve: () => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> } | undefined;
        worklet.port.onmessage = event => {
          if (event.data?.type === "chunk" && event.data.samples instanceof ArrayBuffer) {
            try { deliver(new Int16Array(event.data.samples)); }
            catch (error) { onError(error instanceof Error ? error : new Error("Audio capture failed.")); }
          } else if (event.data?.type === "flushed" && pendingFlush) {
            clearTimeout(pendingFlush.timer);
            pendingFlush.resolve();
            pendingFlush = undefined;
          }
        };
        worklet.onprocessorerror = () => {
          const error = new Error("The microphone audio processor stopped unexpectedly.");
          if (pendingFlush) {
            clearTimeout(pendingFlush.timer);
            pendingFlush.reject(error);
            pendingFlush = undefined;
          }
          onError(error);
        };
        flush = () => new Promise<void>((resolve, reject) => {
          const timer = setTimeout(() => {
            pendingFlush = undefined;
            reject(new Error("The microphone audio processor did not flush in time."));
          }, 1_000);
          pendingFlush = { resolve, reject, timer };
          worklet.port.postMessage({ type: "flush" });
        });
        detach = () => {
          if (pendingFlush) clearTimeout(pendingFlush.timer);
          pendingFlush = undefined;
          worklet.port.onmessage = null;
          worklet.onprocessorerror = null;
          worklet.port.close();
        };
      } else {
        // macOS 12 WKWebView has AudioWorklet, but this bounded fallback keeps
        // microphone cleanup deterministic on older or restricted webviews.
        const processor = context.createScriptProcessor(4096, 1, 1);
        const resampler = new ContinuousPcm16Resampler(context.sampleRate, 16_000, 1_600, deliver);
        processor.onaudioprocess = event => {
          try { resampler.push(event.inputBuffer.getChannelData(0)); }
          catch (error) { onError(error instanceof Error ? error : new Error("Audio capture failed.")); }
        };
        node = processor;
        flush = async () => resampler.flush();
        detach = () => { processor.onaudioprocess = null; };
      }
      source.connect(node);
      node.connect(mute);
      mute.connect(context.destination);
      return new VoiceCapture(stream, context, node, source, mute, onEnded, flush, detach);
    } catch (error) {
      try { source?.disconnect(); } catch { /* best-effort partial setup cleanup */ }
      try { node?.disconnect(); } catch { /* best-effort partial setup cleanup */ }
      try { mute?.disconnect(); } catch { /* best-effort partial setup cleanup */ }
      detach();
      for (const track of stream.getTracks()) {
        track.removeEventListener?.("ended", onEnded);
        track.stop();
      }
      await context?.close().catch(() => undefined);
      throw error;
    }
  }

  async stop(): Promise<void> {
    if (!this.stopping) this.stopping = this.stopOnce();
    return this.stopping;
  }

  private async stopOnce(): Promise<void> {
    let flushError: unknown;
    try { await this.flush(); } catch (error) { flushError = error; }
    this.detach();
    // A detached node must not prevent the remaining resources being released.
    try { this.source.disconnect(); } catch { /* already disconnected */ }
    try { this.node.disconnect(); } catch { /* already disconnected */ }
    try { this.mute.disconnect(); } catch { /* already disconnected */ }
    for (const track of this.stream.getTracks()) {
      track.removeEventListener?.("ended", this.onEnded);
      track.stop();
    }
    await this.context.close().catch(() => undefined);
    if (flushError) throw flushError;
  }
}
