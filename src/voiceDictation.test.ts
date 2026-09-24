import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { VoiceTranscriptPayload } from "./api";
import type { VoiceStartResult } from "./protocol/generated/protocol";
import type { VoiceChunk } from "./voiceCapture";
import { insertDictation, VoiceDictationController, VOICE_QUEUE_BYTES, VOICE_RPC_TIMEOUT_MS,
  VOICE_START_TIMEOUT_MS, type VoiceDraft, type VoiceView } from "./voiceDictation";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const contract: VoiceStartResult = {
  ownerKey: "chat-1", sessionId: "chat-1", voiceSessionId: "voice-1", provider: "codex",
  encoding: "pcm_s16_le", sampleRate: 16_000, channels: 1,
  maxChunkBytes: 65_536, maxSessionBytes: 4 * 1024 * 1024,
};
const pcm = (bytes = 3200): VoiceChunk => ({ data: "A".repeat(Math.ceil(bytes / 3) * 4), samplesPerChannel: bytes / 2 });
function setup() {
  let draft: VoiceDraft = { ownerKey: "chat-1", sessionId: "chat-1", text: "before  after\n", revision: 0, selectionStart: 7, selectionEnd: 7 };
  const transport = {
    start: vi.fn().mockResolvedValue(contract), append: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue(undefined), cancel: vi.fn().mockResolvedValue(undefined),
  };
  const capture = { stop: vi.fn().mockResolvedValue(undefined) };
  let deliver!: (chunk: VoiceChunk) => void;
  let captureError!: (error: Error) => void;
  const startCapture = vi.fn((chunk: typeof deliver, error: typeof captureError) => {
    deliver = chunk; captureError = error;
    return Promise.resolve(capture);
  });
  const changed = vi.fn<(view: VoiceView) => void>();
  const commit = vi.fn();
  const controller = new VoiceDictationController({ transport, capture: startCapture, readDraft: () => draft, changed, commit });
  const event = (kind: VoiceTranscriptPayload["kind"], fields: Partial<VoiceTranscriptPayload> = {}) => {
    controller.receive({ voiceSessionId: "voice-1", ownerKey: "chat-1", sessionId: "chat-1", provider: "codex", kind, ...fields });
  };
  return {
    controller, transport, capture, startCapture, commit, event,
    deliver: (chunk: VoiceChunk = pcm()) => deliver(chunk),
    captureError: (error: Error) => captureError(error),
    edit: (changes: Partial<VoiceDraft>) => { draft = { ...draft, ...changes, revision: draft.revision + 1 }; },
    view: () => changed.mock.calls.at(-1)![0],
    ready: async () => { await controller.start("codex"); event("started"); },
  };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("dictation draft insertion", () => {
  it("replaces only the selected span, preserving surrounding whitespace", () => {
    const draft = { ownerKey: "a", revision: 0, text: "  old text\n", selectionStart: 2, selectionEnd: 5 };
    expect(insertDictation(draft, "new")).toEqual({ text: "  new text\n", caret: 5 });
    expect(insertDictation(draft, " ")).toEqual({ text: draft.text, caret: 5 });
    expect(insertDictation({ ...draft, text: "hello!", selectionStart: 5, selectionEnd: 5 }, "world"))
      .toEqual({ text: "hello world!", caret: 11 });
  });
});

describe("dictation ownership", () => {
  it("handles revisable local speech in a fresh draft without a coding session", async () => {
    const s = setup();
    s.edit({ ownerKey: "fresh", sessionId: undefined });
    s.transport.start.mockResolvedValue({ ...contract, ownerKey: "fresh", sessionId: null, provider: "local" });
    const scope = { ownerKey: "fresh", sessionId: null, provider: "local" as const };
    await s.controller.start("local");
    expect(s.transport.start).toHaveBeenCalledWith("fresh", "local", undefined);
    s.event("started", scope);
    s.event("partial", { ...scope, text: "write a cash" });
    s.event("partial", { ...scope, text: "write a cache" });
    expect(s.view().preview).toBe("write a cache");
    expect(s.commit).not.toHaveBeenCalled();
    s.event("final", { ...scope, text: "write a cache" });
    s.event("closed", scope);
    expect(s.commit).toHaveBeenCalledWith("before write a cache after\n", 20);
  });

  it("accepts an empty final that retracts an inaccurate partial", async () => {
    const s = setup();
    await s.ready();
    s.event("partial", { text: "noise" });
    s.event("final", { text: "" });
    s.event("closed");
    expect(s.view().state).toBe("idle");
    expect(s.commit).not.toHaveBeenCalled();
  });

  it("rejects a wrong owner even when the voice and coding session IDs match", async () => {
    const s = setup();
    await s.ready();
    s.event("partial", { ownerKey: "other", text: "wrong" });
    s.event("closed", { ownerKey: "other" });
    expect(s.view()).toEqual({ state: "recording", preview: "" });
    expect(s.commit).not.toHaveBeenCalled();
    s.controller.cancel();
  });

  it("waits for ready, previews without editing, and commits only on terminal completion", async () => {
    const s = setup();
    await s.controller.start("codex");
    s.deliver();
    expect(s.view().state).toBe("starting");
    expect(s.transport.append).not.toHaveBeenCalled();
    s.event("started");
    await vi.advanceTimersByTimeAsync(0);
    expect(s.transport.append).toHaveBeenCalledWith("voice-1", 0, pcm().data, 1600);
    s.event("delta", { text: "hello" });
    expect(s.view().preview).toBe("hello");
    expect(s.commit).not.toHaveBeenCalled();
    s.event("final", { text: "hello world" });
    await s.controller.stop();
    expect(s.view().state).toBe("stopping");
    expect(s.commit).not.toHaveBeenCalled();
    s.event("closed");
    expect(s.commit).toHaveBeenCalledWith("before hello world after\n", 18);
    expect(s.capture.stop).toHaveBeenCalledTimes(1);
    expect(s.view().state).toBe("idle");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("cleans a permission result after cancellation without starting a provider", async () => {
    const s = setup();
    const permission = deferred<typeof s.capture>();
    s.startCapture.mockReturnValueOnce(permission.promise);
    const start = s.controller.start("codex");
    s.controller.cancel();
    permission.resolve(s.capture);
    await start;
    expect(s.capture.stop).toHaveBeenCalledTimes(1);
    expect(s.transport.start).not.toHaveBeenCalled();
  });

  it("a late old start cancels only itself, never a replacement take", async () => {
    const s = setup();
    const started = deferred<VoiceStartResult>();
    s.transport.start.mockReturnValueOnce(started.promise);
    const oldStart = s.controller.start("codex");
    await vi.advanceTimersByTimeAsync(0);
    s.controller.cancel();
    const newCapture = { stop: vi.fn().mockResolvedValue(undefined) };
    s.startCapture.mockResolvedValueOnce(newCapture);
    s.transport.start.mockResolvedValueOnce({ ...contract, voiceSessionId: "voice-2" });
    await s.controller.start("codex");
    started.resolve(contract);
    await oldStart;
    expect(s.transport.cancel).toHaveBeenCalledWith("voice-1");
    expect(newCapture.stop).not.toHaveBeenCalled();
    s.event("started", { voiceSessionId: "voice-2" });
    s.event("error", { error: "old error" });
    expect(s.view().state).toBe("recording");
    s.controller.cancel();
  });

  it("correlates early ready events after start resolves", async () => {
    const s = setup();
    const started = deferred<VoiceStartResult>();
    s.transport.start.mockReturnValueOnce(started.promise);
    const starting = s.controller.start("codex");
    await vi.advanceTimersByTimeAsync(0);
    s.event("error", { voiceSessionId: "other", error: "stale" });
    s.event("started");
    started.resolve(contract);
    await starting;
    expect(s.view().state).toBe("recording");
    s.controller.cancel();
  });

  it.each(["error", "closed"] as const)("releases capture on provider %s", async kind => {
    const s = setup();
    await s.ready();
    s.event(kind);
    expect(s.capture.stop).toHaveBeenCalledTimes(1);
    expect(s.controller.active).toBe(false);
    s.deliver();
    s.event("delta", { text: "late" });
    expect(s.transport.append).not.toHaveBeenCalled();
    expect(s.commit).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each(["revision", "owner"] as const)("does not overwrite a draft after its %s changes", async mode => {
    const s = setup();
    await s.ready();
    if (mode === "owner") s.edit({ ownerKey: "chat-2" });
    else { s.edit({ text: "other" }); s.edit({ text: "before  after\n" }); }
    s.event("final", { text: "stale" });
    s.event("closed");
    expect(s.view().state).toBe("error");
    expect(s.commit).not.toHaveBeenCalled();
    expect(s.capture.stop).toHaveBeenCalledTimes(1);
  });

  it("does not commit incomplete partials", async () => {
    const s = setup();
    await s.ready();
    s.event("delta", { text: "unfinished" });
    s.event("closed");
    expect(s.view().error).toMatch(/finalized/);
    expect(s.commit).not.toHaveBeenCalled();
  });

  it("stops on device loss", async () => {
    const s = setup();
    await s.ready();
    s.captureError(new Error("device disconnected"));
    expect(s.view().error).toBe("device disconnected");
    expect(s.capture.stop).toHaveBeenCalledTimes(1);
  });
});

describe("dictation resource bounds", () => {
  it("bounds queued audio while readiness is pending", async () => {
    const s = setup();
    await s.controller.start("codex");
    s.deliver(pcm(VOICE_QUEUE_BYTES)); s.deliver();
    expect(s.view().error).toMatch(/keep up/);
    expect(s.capture.stop).toHaveBeenCalledTimes(1);
    expect(s.transport.append).not.toHaveBeenCalled();
  });

  it("includes in-flight audio in its slow-consumer bound", async () => {
    const s = setup();
    const append = deferred<void>();
    s.transport.append.mockReturnValueOnce(append.promise);
    await s.ready();
    s.deliver(pcm(VOICE_QUEUE_BYTES)); s.deliver();
    expect(s.view().error).toMatch(/keep up/);
    append.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(s.transport.append).toHaveBeenCalledTimes(1);
  });

  it("drains capture's final frame before asking the provider to stop", async () => {
    const s = setup();
    await s.ready();
    s.capture.stop.mockImplementationOnce(async () => { s.deliver(); });
    await s.controller.stop();
    expect(s.transport.append).toHaveBeenCalledTimes(1);
    expect(s.transport.append.mock.invocationCallOrder[0]).toBeLessThan(s.transport.stop.mock.invocationCallOrder[0]);
    s.event("closed");
  });

  it("times out readiness despite a successful start RPC write", async () => {
    const s = setup();
    await s.controller.start("codex");
    await vi.advanceTimersByTimeAsync(VOICE_START_TIMEOUT_MS);
    expect(s.view().error).toMatch(/ready/);
    expect(s.capture.stop).toHaveBeenCalledTimes(1);
    expect(s.transport.cancel).toHaveBeenCalledWith("voice-1");
  });

  it("a timed-out append's late rejection cannot affect another take", async () => {
    const s = setup();
    const append = deferred<void>();
    s.transport.append.mockReturnValueOnce(append.promise);
    await s.ready(); s.deliver();
    await vi.advanceTimersByTimeAsync(VOICE_RPC_TIMEOUT_MS);
    expect(s.view().error).toMatch(/timed out/);
    s.transport.start.mockResolvedValueOnce({ ...contract, voiceSessionId: "voice-2" });
    await s.controller.start("codex");
    s.event("started", { voiceSessionId: "voice-2" });
    append.reject(new Error("old rejection"));
    await vi.advanceTimersByTimeAsync(0);
    expect(s.view().state).toBe("recording");
    s.controller.cancel();
  });

  it("times out missing terminal events and cleans once", async () => {
    const s = setup();
    await s.ready(); await s.controller.stop();
    await vi.advanceTimersByTimeAsync(VOICE_RPC_TIMEOUT_MS);
    expect(s.view().error).toMatch(/finish in time/);
    expect(s.capture.stop).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("rejects an incompatible negotiated format", async () => {
    const s = setup();
    s.transport.start.mockResolvedValueOnce({ ...contract, sampleRate: 48_000 });
    await s.controller.start("codex");
    expect(s.view().error).toMatch(/unsupported audio contract/);
    expect(s.transport.cancel).toHaveBeenCalledWith("voice-1");
  });
});
