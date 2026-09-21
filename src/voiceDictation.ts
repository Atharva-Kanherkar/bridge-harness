import type { VoiceStartResult, VoiceProviderId } from "./protocol/generated/protocol";
import type { VoiceTranscriptPayload as VoiceTranscriptEvent } from "./api";
import { applyVoiceTranscriptDraft, type VoiceChunk, type VoiceDraftTranscript } from "./voiceCapture";

export type VoiceState = "idle" | "starting" | "recording" | "stopping" | "error";
export type VoiceView = { state: VoiceState; preview: string; error?: string };
export type VoiceDraft = {
  ownerKey: string;
  sessionId?: string;
  text: string;
  revision: number;
  selectionStart: number;
  selectionEnd: number;
};
export type VoiceCaptureHandle = { stop(): Promise<void> };
export type VoiceTransport = {
  start(ownerKey: string, provider: VoiceProviderId, sessionId?: string): Promise<VoiceStartResult>;
  append(id: string, sequence: number, data: string, samples: number): Promise<void>;
  stop(id: string): Promise<void>;
  cancel(id: string): Promise<void>;
};
type Dependencies = {
  transport: VoiceTransport;
  capture(onChunk: (chunk: VoiceChunk) => void, onError: (error: Error) => void): Promise<VoiceCaptureHandle>;
  readDraft(): VoiceDraft;
  commit(text: string, caret: number): void;
  changed(view: VoiceView): void;
};

// Bounds include an in-flight frame. Never accumulate minutes of audio behind
// a slow RPC or while the user/provider is still preparing a recording.
export const VOICE_QUEUE_BYTES = 64_000;
export const VOICE_START_TIMEOUT_MS = 30_000;
export const VOICE_RPC_TIMEOUT_MS = 10_000;
export const VOICE_MAX_DURATION_MS = 120_000;
const MAX_EARLY_EVENTS = 32;
const MAX_TRANSCRIPT_CHARS = 64_000;

type Take = {
  draft: VoiceDraft;
  provider: VoiceProviderId;
  state: VoiceState;
  id?: string;
  contract?: VoiceStartResult;
  capture?: VoiceCaptureHandle;
  releasing?: Promise<void>;
  ready: boolean;
  queue: VoiceChunk[];
  queuedBytes: number;
  totalBytes: number;
  sequence: number;
  pumping?: Promise<void>;
  transcript: VoiceDraftTranscript;
  early: VoiceTranscriptEvent[];
  earlyChars: number;
  timers: Set<ReturnType<typeof setTimeout>>;
  startTimer?: ReturnType<typeof setTimeout>;
};

function sameDraft(left: VoiceDraft, right: VoiceDraft): boolean {
  return left.ownerKey === right.ownerKey && left.sessionId === right.sessionId && left.revision === right.revision && left.text === right.text;
}

/** Preserve all surrounding text and replace exactly the captured selection. */
export function insertDictation(draft: VoiceDraft, transcript: string): { text: string; caret: number } {
  const words = transcript.trim();
  if (!words) return { text: draft.text, caret: draft.selectionEnd };
  const start = Math.max(0, Math.min(draft.text.length, draft.selectionStart));
  const end = Math.max(start, Math.min(draft.text.length, draft.selectionEnd));
  const before = draft.text.slice(0, start);
  const after = draft.text.slice(end);
  const lead = before && !/[\s([{]$/.test(before) ? " " : "";
  const trail = after && !/^[\s,.;:!?)\]}]/.test(after) ? " " : "";
  return { text: before + lead + words + trail + after, caret: before.length + lead.length + words.length };
}

/** A take object is its generation token. No continuation may act on a newer take. */
export class VoiceDictationController {
  private take?: Take;

  constructor(private readonly deps: Dependencies) {}

  get active(): boolean { return this.take !== undefined; }

  draftChanged(): void {
    if (this.take && !sameDraft(this.take.draft, this.deps.readDraft())) {
      this.fail(this.take, "The draft changed. Dictation was cancelled without replacing your text.");
    }
  }

  async start(provider: VoiceProviderId): Promise<void> {
    if (this.take) return;
    const take: Take = {
      draft: this.deps.readDraft(), provider, state: "starting", ready: false,
      queue: [], queuedBytes: 0, totalBytes: 0, sequence: 0,
      transcript: { finalized: "", provisional: "" }, early: [], earlyChars: 0, timers: new Set(),
    };
    this.take = take;
    this.publish(take);
    take.startTimer = this.timer(take, VOICE_START_TIMEOUT_MS, () => this.fail(take, "Dictation did not become ready. Check microphone permission and retry."));
    try {
      const capture = await this.deps.capture(chunk => this.enqueue(take, chunk), error => this.fail(take, error.message));
      if (!this.owns(take)) { void capture.stop().catch(() => undefined); return; }
      take.capture = capture;
      const started = await this.deps.transport.start(take.draft.ownerKey, provider, take.draft.sessionId);
      if (!this.owns(take)) { void this.deps.transport.cancel(started.voiceSessionId).catch(() => undefined); return; }
      take.id = started.voiceSessionId;
      take.contract = started;
      if (started.ownerKey !== take.draft.ownerKey || (started.sessionId ?? undefined) !== take.draft.sessionId || started.provider !== provider ||
          started.encoding !== "pcm_s16_le" || started.sampleRate !== 16_000 || started.channels !== 1 ||
          started.maxChunkBytes <= 0 || started.maxSessionBytes <= 0) {
        throw new Error("The dictation provider returned an unsupported audio contract.");
      }
      // A notification can arrive before the start RPC resolves. Buffer only a
      // small amount, then correlate by the returned opaque ID before applying.
      const early = take.early;
      take.early = [];
      take.earlyChars = 0;
      for (const event of early) this.receive(event);
    } catch (error) {
      this.fail(take, error instanceof Error ? error.message : "Could not start dictation.");
    }
  }

  receive(event: VoiceTranscriptEvent): void {
    const take = this.take;
    if (!take || event.ownerKey !== take.draft.ownerKey || (event.sessionId ?? undefined) !== take.draft.sessionId || event.provider !== take.provider || !this.owns(take)) return;
    if (!take.id) {
      const chars = (event.text?.length ?? 0) + (event.error?.length ?? 0);
      if (take.early.length >= MAX_EARLY_EVENTS || take.earlyChars + chars > MAX_TRANSCRIPT_CHARS) {
        this.fail(take, "Too many dictation events arrived before startup completed.");
        return;
      }
      take.early.push(event);
      take.earlyChars += chars;
      return;
    }
    if (event.voiceSessionId !== take.id) return;
    if (event.kind === "error") { this.fail(take, event.error ?? "Dictation failed."); return; }
    if (event.kind === "closed") { this.finish(take); return; }
    if (event.kind === "started" && !take.ready) {
      take.ready = true;
      take.state = "recording";
      if (take.startTimer) { clearTimeout(take.startTimer); take.timers.delete(take.startTimer); }
      this.timer(take, VOICE_MAX_DURATION_MS, () => { void this.stop(); });
      this.publish(take);
      this.pump(take);
    }
    if ((event.kind === "partial" || event.kind === "delta" || event.kind === "final") && typeof event.text === "string") {
      const transcript = applyVoiceTranscriptDraft(take.transcript, event.kind, event.text);
      if (transcript.finalized.length + transcript.provisional.length > MAX_TRANSCRIPT_CHARS) {
        this.fail(take, "The dictation transcript exceeded its safety limit.");
        return;
      }
      take.transcript = transcript;
      this.publish(take);
    }
  }

  async stop(): Promise<void> {
    const take = this.take;
    if (!take || take.state === "stopping") return;
    // Releasing the button during a permission/startup prompt cancels that
    // generation. Its eventual capture/RPC result must only clean itself up.
    if (!take.ready || !take.id) { this.cancel(); return; }
    take.state = "stopping";
    this.publish(take);
    this.timer(take, VOICE_RPC_TIMEOUT_MS, () => this.fail(take, "Dictation did not finish in time. Your draft is unchanged."));
    try {
      await this.releaseCapture(take);
      if (!this.owns(take)) return;
      // stop() may flush a last PCM frame synchronously or asynchronously.
      while (take.pumping && this.owns(take)) await take.pumping;
      if (!this.owns(take)) return;
      await this.deps.transport.stop(take.id);
      // Completion is the provider's terminal notification, not a pipe write.
    } catch (error) {
      this.fail(take, error instanceof Error ? error.message : "Could not finish dictation.");
    }
  }

  cancel(): void {
    const take = this.take;
    if (!take) return;
    this.end(take, true);
    this.deps.changed({ state: "idle", preview: "" });
  }

  private owns(take: Take): boolean {
    if (this.take !== take) return false;
    if (!sameDraft(take.draft, this.deps.readDraft())) {
      this.fail(take, "The draft changed. Dictation was cancelled without replacing your text.");
      return false;
    }
    return true;
  }

  private enqueue(take: Take, chunk: VoiceChunk): void {
    if (!this.owns(take)) return;
    const bytes = chunk.samplesPerChannel * 2;
    if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > VOICE_QUEUE_BYTES ||
        chunk.data.length > Math.ceil(bytes / 3) * 4) {
      this.fail(take, "Audio capture produced an invalid frame.");
      return;
    }
    if (take.queuedBytes + bytes > VOICE_QUEUE_BYTES) {
      this.fail(take, "Dictation cannot keep up with the microphone. Please retry.");
      return;
    }
    take.queue.push(chunk);
    take.queuedBytes += bytes;
    this.pump(take);
  }

  private pump(take: Take): void {
    if (!take.queue.length || take.pumping || !take.ready || !take.id || !take.contract || !this.owns(take)) return;
    const id = take.id;
    const contract = take.contract;
    take.pumping = (async () => {
      while (take.queue.length && this.owns(take)) {
        const chunk = take.queue.shift()!;
        const bytes = chunk.samplesPerChannel * 2;
        if (bytes > contract.maxChunkBytes || take.totalBytes + bytes > contract.maxSessionBytes) {
          this.fail(take, "Dictation reached the provider's audio limit. Your draft is unchanged.");
          return;
        }
        const timeout = this.timer(take, VOICE_RPC_TIMEOUT_MS, () => this.fail(take, "Dictation audio delivery timed out."));
        try {
          await this.deps.transport.append(id, take.sequence, chunk.data, chunk.samplesPerChannel);
        } finally {
          clearTimeout(timeout);
          take.timers.delete(timeout);
        }
        if (!this.owns(take)) return;
        take.queuedBytes -= bytes;
        take.totalBytes += bytes;
        take.sequence += 1;
      }
    })().catch(error => this.fail(take, error instanceof Error ? error.message : "Dictation audio delivery failed."))
      .finally(() => {
        take.pumping = undefined;
        if (this.take === take && take.queue.length) this.pump(take);
      });
  }

  private releaseCapture(take: Take): Promise<void> {
    if (!take.releasing) take.releasing = take.capture?.stop() ?? Promise.resolve();
    return take.releasing;
  }

  private finish(take: Take): void {
    if (!this.owns(take)) return;
    if (take.transcript.provisional.trim()) {
      this.fail(take, "Dictation closed before its transcript was finalized. Your draft is unchanged.");
      return;
    }
    const final = take.transcript.finalized.trim();
    this.end(take, false);
    if (final) {
      const inserted = insertDictation(take.draft, final);
      this.deps.commit(inserted.text, inserted.caret);
    }
    this.deps.changed({ state: "idle", preview: "" });
  }

  private fail(take: Take, error: string): void {
    if (this.take !== take) return;
    this.end(take, true);
    this.deps.changed({ state: "error", preview: "", error });
  }

  private end(take: Take, cancelProvider: boolean): void {
    this.take = undefined;
    for (const timer of take.timers) clearTimeout(timer);
    take.timers.clear();
    take.queue = [];
    take.early = [];
    void this.releaseCapture(take).catch(() => undefined);
    if (cancelProvider && take.id) void this.deps.transport.cancel(take.id).catch(() => undefined);
  }

  private timer(take: Take, delay: number, callback: () => void): ReturnType<typeof setTimeout> {
    const timer = setTimeout(() => {
      take.timers.delete(timer);
      if (this.take === take) callback();
    }, delay);
    take.timers.add(timer);
    return timer;
  }

  private publish(take: Take): void {
    this.deps.changed({ state: take.state, preview: [take.transcript.finalized, take.transcript.provisional].filter(Boolean).join(" ") });
  }
}
