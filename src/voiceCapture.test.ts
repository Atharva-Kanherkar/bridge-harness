import { afterEach, describe, expect, it, vi } from "vitest";
import {
  VoiceCapture,
  applyVoiceTranscriptDraft,
  pcm16ToBase64,
  renderVoiceDraft,
  resampleToPcm16,
} from "./voiceCapture";

afterEach(() => vi.unstubAllGlobals());

describe("voice capture conversion", () => {
  it("downsamples and clamps mono float audio to signed PCM16", () => {
    expect(Array.from(resampleToPcm16(new Float32Array([-2, -1, 0.5, 2]), 32_000, 16_000)))
      .toEqual([-32768, 32767]);
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
});

describe("voice draft projection", () => {
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
