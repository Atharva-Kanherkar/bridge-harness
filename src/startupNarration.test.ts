import { describe, expect, it } from "vitest";
import { computeNarration, type NarrationInput } from "./startupNarration";

const base: NarrationInput = {
  hasPendingWork: true,
  streaming: false,
  harnessName: "OpenCode",
  modelName: "Sonnet",
  switchingToLabel: null,
  latestPhase: null,
  startedAt: 0,
  streamStartedAt: null,
  now: 0,
  reducedMotion: false,
};

describe("computeNarration", () => {
  it("is unmounted with no pending work", () => {
    const view = computeNarration({ ...base, hasPendingWork: false });
    expect(view.mounted).toBe(false);
  });

  it("labels each observed phase, harness-named", () => {
    expect(computeNarration({ ...base, latestPhase: "spawning" }).label).toBe("Starting OpenCode…");
    expect(computeNarration({ ...base, latestPhase: "handshake" }).label).toBe("Waiting for OpenCode to answer…");
    expect(computeNarration({ ...base, latestPhase: "session_open" }).label).toBe("Opening the session…");
  });

  it("defaults to the reading-your-message label once no cold-start phase is in flight", () => {
    const view = computeNarration({ ...base, latestPhase: null });
    expect(view.label).toBe("Sonnet is reading your message…");
  });

  it("hides the elapsed counter before 2s and shows it from 2s on", () => {
    const before = computeNarration({ ...base, startedAt: 0, now: 1999 });
    expect(before.showElapsed).toBe(false);
    const at = computeNarration({ ...base, startedAt: 0, now: 2000 });
    expect(at.showElapsed).toBe(true);
    expect(at.elapsedSeconds).toBe(2);
  });

  // The switch state is Bridge's own activity: it mounts without pending
  // work, outranks every phase label, and vanishes the moment it is cleared.
  it("narrates a model switch even with no pending work", () => {
    const view = computeNarration({ ...base, hasPendingWork: false, switchingToLabel: "Opus" });
    expect(view.mounted).toBe(true);
    expect(view.label).toBe("Switching to Opus…");
  });

  it("lets the switch outrank a cold-start phase and the reading label", () => {
    const view = computeNarration({ ...base, latestPhase: "handshake", switchingToLabel: "GPT Luna" });
    expect(view.label).toBe("Switching to GPT Luna…");
  });

  it("shows the switch's elapsed counter from 2s, like every other wait", () => {
    expect(computeNarration({ ...base, switchingToLabel: "Opus", startedAt: 0, now: 1999 }).showElapsed).toBe(false);
    const at = computeNarration({ ...base, switchingToLabel: "Opus", startedAt: 0, now: 2400 });
    expect(at.showElapsed).toBe(true);
    expect(at.elapsedSeconds).toBe(2);
  });

  it("unmounts as soon as the switch clears with nothing pending", () => {
    const view = computeNarration({ ...base, hasPendingWork: false, switchingToLabel: null });
    expect(view.mounted).toBe(false);
  });

  it("collapses the label the instant streaming starts, but keeps the row mounted", () => {
    const view = computeNarration({ ...base, streaming: true, streamStartedAt: 0, now: 0 });
    expect(view.mounted).toBe(true);
    expect(view.collapsed).toBe(true);
    expect(view.label).toBe("");
  });

  it("unmounts 2s after the first token, not an instant sooner", () => {
    const lingering = computeNarration({ ...base, streaming: true, streamStartedAt: 0, now: 1999 });
    expect(lingering.mounted).toBe(true);
    const gone = computeNarration({ ...base, streaming: true, streamStartedAt: 0, now: 2000 });
    expect(gone.mounted).toBe(false);
  });

  it("never remounts on hasPendingWork alone once streaming is lingering", () => {
    // hasPendingWork stays true through the whole lingering window in
    // practice; this pins that the mount decision comes from the lingering
    // clock, not from streaming flipping true and false around it.
    const view = computeNarration({ ...base, hasPendingWork: true, streaming: true, streamStartedAt: 500, now: 1200 });
    expect(view.mounted).toBe(true);
  });

  it("passes the reduced-motion flag through even when unmounted", () => {
    const view = computeNarration({ ...base, hasPendingWork: false, reducedMotion: true });
    expect(view.reducedMotion).toBe(true);
  });
});
