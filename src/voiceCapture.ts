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
  kind: "delta" | "final",
  text: string,
): VoiceDraftTranscript {
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
  if (inputRate <= 0 || targetRate <= 0) throw new Error("Audio sample rates must be positive");
  if (input.length === 0) return new Int16Array();
  const ratio = inputRate / targetRate;
  const length = Math.max(1, Math.floor(input.length / ratio));
  const output = new Int16Array(length);
  for (let index = 0; index < length; index += 1) {
    const start = Math.floor(index * ratio);
    const end = Math.max(start + 1, Math.min(input.length, Math.floor((index + 1) * ratio)));
    let sum = 0;
    for (let source = start; source < end; source += 1) sum += input[source];
    const value = Math.max(-1, Math.min(1, sum / (end - start)));
    output[index] = value < 0 ? Math.round(value * 0x8000) : Math.round(value * 0x7fff);
  }
  return output;
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
  private stopped = false;

  private constructor(
    private readonly stream: MediaStream,
    private readonly context: AudioContext,
    private readonly processor: ScriptProcessorNode,
    private readonly source: MediaStreamAudioSourceNode,
    private readonly mute: GainNode,
    private readonly onEnded: () => void,
  ) {}

  static async start(onChunk: (chunk: VoiceChunk) => void, onError: (error: Error) => void = () => {}): Promise<VoiceCapture> {
    if (!navigator.mediaDevices?.getUserMedia) throw new Error("Microphone capture is not supported by this app runtime.");
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    });
    let context: AudioContext | undefined;
    let source: MediaStreamAudioSourceNode | undefined;
    let processor: ScriptProcessorNode | undefined;
    let mute: GainNode | undefined;
    const onEnded = () => onError(new Error("The microphone disconnected. Reconnect it and retry dictation."));
    try {
      for (const track of stream.getTracks()) track.addEventListener?.("ended", onEnded);
      context = new AudioContext();
      source = context.createMediaStreamSource(stream);
      // ScriptProcessor remains the most widely supported capture primitive in
      // WKWebView. It never drives UI; it only converts bounded mono buffers.
      processor = context.createScriptProcessor(4096, 1, 1);
      mute = context.createGain();
      mute.gain.value = 0;
      processor.onaudioprocess = event => {
        try {
          const pcm = resampleToPcm16(event.inputBuffer.getChannelData(0), context!.sampleRate);
          if (pcm.length) onChunk({ data: pcm16ToBase64(pcm), samplesPerChannel: pcm.length });
        } catch (error) {
          onError(error instanceof Error ? error : new Error("Audio capture failed."));
        }
      };
      source.connect(processor);
      processor.connect(mute);
      mute.connect(context.destination);
      return new VoiceCapture(stream, context, processor, source, mute, onEnded);
    } catch (error) {
      try { source?.disconnect(); } catch { /* best-effort partial setup cleanup */ }
      try { processor?.disconnect(); } catch { /* best-effort partial setup cleanup */ }
      try { mute?.disconnect(); } catch { /* best-effort partial setup cleanup */ }
      for (const track of stream.getTracks()) {
        track.removeEventListener?.("ended", onEnded);
        track.stop();
      }
      await context?.close().catch(() => undefined);
      throw error;
    }
  }

  async stop(): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;
    this.processor.onaudioprocess = null;
    // A detached node must not prevent the remaining resources being released.
    try { this.source.disconnect(); } catch { /* already disconnected */ }
    try { this.processor.disconnect(); } catch { /* already disconnected */ }
    try { this.mute.disconnect(); } catch { /* already disconnected */ }
    for (const track of this.stream.getTracks()) {
      track.removeEventListener?.("ended", this.onEnded);
      track.stop();
    }
    await this.context.close().catch(() => undefined);
  }
}
