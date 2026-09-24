import { afterEach, describe, expect, it, vi } from "vitest";
import {
  VoiceCapture,
  applyVoiceTranscriptDraft,
  pcm16ToBase64,
  renderVoiceDraft,
  resampleToPcm16,
} from "./voiceCapture";
import { ContinuousPcm16Resampler } from "./voiceResampler";

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("voice capture conversion", () => {
  it("downsamples, flushes the tail, and clamps mono float audio to signed PCM16", () => {
    expect(Array.from(resampleToPcm16(new Float32Array([-2, -1, 0.5, 2]), 32_000, 16_000)))
      .toEqual([-32768, 16384, 32767]);
  });

  it("encodes the little-endian bytes without changing the sample payload", () => {
    vi.stubGlobal("btoa", (value: string) => Buffer.from(value, "binary").toString("base64"));
    const samples = new Int16Array([0x1234, -2]);
    expect(Array.from(Buffer.from(pcm16ToBase64(samples), "base64")))
      .toEqual([0x34, 0x12, 0xfe, 0xff]);
  });

  it("does not invent a sample for an empty input buffer", () => {
    expect(resampleToPcm16(new Float32Array(), 48_000)).toHaveLength(0);
  });

  it.each([16_000, 44_100, 48_000])("keeps continuous phase across arbitrary %i Hz input buffers", inputRate => {
    const input = Float32Array.from({ length: 997 }, (_, index) => Math.sin(index / 19));
    const run = (boundaries: number[]) => {
      const output: number[] = [];
      const resampler = new ContinuousPcm16Resampler(inputRate, 16_000, 73, chunk => output.push(...chunk));
      let offset = 0;
      for (const length of boundaries) {
        resampler.push(input.slice(offset, offset + length));
        offset += length;
      }
      if (offset < input.length) resampler.push(input.slice(offset));
      resampler.flush();
      return output;
    };
    expect(run([127, 3, 251, 1, 89])).toEqual(run([997]));
  });

  it("flushes a take shorter than one target sample interval", () => {
    const output: number[] = [];
    const resampler = new ContinuousPcm16Resampler(48_000, 16_000, 1_600, chunk => output.push(...chunk));
    resampler.push(new Float32Array([0.25]));
    resampler.flush();
    expect(output).toEqual([8192]);
  });
});

describe("voice draft projection", () => {
  it("replaces revised hypotheses, including empty retractions, without duplication", () => {
    let value = { finalized: "", provisional: "" };
    value = applyVoiceTranscriptDraft(value, "partial", "write a cash");
    value = applyVoiceTranscriptDraft(value, "partial", "write a cache");
    expect(value.provisional).toBe("write a cache");
    value = applyVoiceTranscriptDraft(value, "partial", "");
    expect(value.provisional).toBe("");
    value = applyVoiceTranscriptDraft(value, "final", "write a cache");
    expect(value).toEqual({ finalized: "write a cache", provisional: "" });
  });
  it("preserves the existing draft and replaces only the provisional phrase", () => {
    let transcript = applyVoiceTranscriptDraft(
      { finalized: "", provisional: "" },
      "delta",
      "hello wor",
    );
    expect(renderVoiceDraft("existing draft", transcript)).toBe("existing draft hello wor");

    transcript = applyVoiceTranscriptDraft(transcript, "final", "hello world");
    expect(renderVoiceDraft("existing draft", transcript)).toBe("existing draft hello world");

    transcript = applyVoiceTranscriptDraft(transcript, "delta", " again");
    expect(renderVoiceDraft("existing draft", transcript)).toBe("existing draft hello world again");
  });
});

describe("voice capture lifecycle", () => {
  function captureMocks() {
    const track = { stop: vi.fn() };
    const stream = { getTracks: () => [track] };
    const source = { connect: vi.fn(), disconnect: vi.fn() };
    const processor = { onaudioprocess: null, connect: vi.fn(), disconnect: vi.fn() };
    const mute = { gain: { value: 1 }, connect: vi.fn(), disconnect: vi.fn() };
    const context = {
      sampleRate: 48_000,
      destination: {},
      createMediaStreamSource: vi.fn(() => source),
      createScriptProcessor: vi.fn(() => processor),
      createGain: vi.fn(() => mute),
      close: vi.fn().mockResolvedValue(undefined),
    };
    vi.stubGlobal("navigator", {
      mediaDevices: { getUserMedia: vi.fn().mockResolvedValue(stream) },
    });
    vi.stubGlobal("AudioContext", vi.fn(() => context));
    return { context, mute, processor, source, stream, track };
  }

  function workletMocks() {
    const mocks = captureMocks();
    const port = { onmessage: null as ((event: MessageEvent) => void) | null, postMessage: vi.fn(), close: vi.fn() };
    const worklet = { port, onprocessorerror: null as (() => void) | null, connect: vi.fn(), disconnect: vi.fn() };
    Object.assign(mocks.context, { audioWorklet: { addModule: vi.fn().mockResolvedValue(undefined) } });
    vi.stubGlobal("AudioWorkletNode", vi.fn(() => worklet));
    return { ...mocks, port, worklet };
  }

  it("prefers the packaged worklet and waits for its tail acknowledgement before releasing the mic", async () => {
    const mocks = workletMocks();
    const chunks = vi.fn();
    const capture = await VoiceCapture.start(chunks);
    const stopping = capture.stop();
    expect(mocks.port.postMessage).toHaveBeenCalledWith({ type: "flush" });
    expect(mocks.track.stop).not.toHaveBeenCalled();

    const samples = Int16Array.from([1, -2, 3]);
    mocks.port.onmessage?.({ data: { type: "chunk", samples: samples.buffer } } as MessageEvent);
    mocks.port.onmessage?.({ data: { type: "flushed" } } as MessageEvent);
    await stopping;

    expect(chunks).toHaveBeenCalledWith({ data: expect.any(String), samplesPerChannel: 3 });
    expect(mocks.track.stop).toHaveBeenCalledTimes(1);
    expect(mocks.worklet.disconnect).toHaveBeenCalledTimes(1);
    expect(mocks.port.close).toHaveBeenCalledTimes(1);
  });

  it("cancels a pending flush and still releases the mic when the worklet crashes", async () => {
    vi.useFakeTimers();
    const mocks = workletMocks();
    const onError = vi.fn();
    const capture = await VoiceCapture.start(vi.fn(), onError);
    const stopping = capture.stop();
    expect(vi.getTimerCount()).toBe(1);

    mocks.worklet.onprocessorerror?.();
    await expect(stopping).rejects.toThrow("audio processor stopped unexpectedly");

    expect(vi.getTimerCount()).toBe(0);
    expect(onError).toHaveBeenCalledWith(expect.objectContaining({
      message: "The microphone audio processor stopped unexpectedly.",
    }));
    expect(mocks.track.stop).toHaveBeenCalledTimes(1);
    expect(mocks.context.close).toHaveBeenCalledTimes(1);
  });

  it("releases tracks and audio nodes exactly once", async () => {
    const mocks = captureMocks();
    const capture = await VoiceCapture.start(vi.fn());
    await capture.stop();
    await capture.stop();

    expect(mocks.track.stop).toHaveBeenCalledTimes(1);
    expect(mocks.source.disconnect).toHaveBeenCalledTimes(1);
    expect(mocks.processor.disconnect).toHaveBeenCalledTimes(1);
    expect(mocks.mute.disconnect).toHaveBeenCalledTimes(1);
    expect(mocks.context.close).toHaveBeenCalledTimes(1);
  });

  it("releases the microphone when audio graph setup fails", async () => {
    const mocks = captureMocks();
    mocks.context.createMediaStreamSource.mockImplementation(() => {
      throw new Error("audio graph unavailable");
    });

    await expect(VoiceCapture.start(vi.fn())).rejects.toThrow("audio graph unavailable");
    expect(mocks.track.stop).toHaveBeenCalledTimes(1);
    expect(mocks.context.close).toHaveBeenCalledTimes(1);
  });

  it("still releases tracks when disconnecting a node throws", async () => {
    const mocks = captureMocks();
    mocks.source.disconnect.mockImplementation(() => { throw new Error("already detached"); });
    const capture = await VoiceCapture.start(vi.fn());
    await capture.stop();
    expect(mocks.track.stop).toHaveBeenCalledTimes(1);
    expect(mocks.context.close).toHaveBeenCalledTimes(1);
  });
});
