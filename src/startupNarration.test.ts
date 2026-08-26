import { describe, expect, it } from "vitest";
import { computeNarration, type NarrationInput } from "./startupNarration";

const base: NarrationInput = {
  hasPendingWork: true,
  streaming: false,
  harnessName: "OpenCode",
  modelName: "Sonnet",
  firstLaunch: false,
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

  it("shows the first-launch note only while narrating and only for a first launch", () => {
    const firstLaunch = computeNarration({ ...base, firstLaunch: true });
    expect(firstLaunch.showFirstLaunchNote).toBe(true);
    const notFirstLaunch = computeNarration({ ...base, firstLaunch: false });
    expect(notFirstLaunch.showFirstLaunchNote).toBe(false);
  });

  it("collapses the label the instant streaming starts, but keeps the row mounted", () => {
    const view = computeNarration({ ...base, streaming: true, streamStartedAt: 0, now: 0 });
    expect(view.mounted).toBe(true);
    expect(view.collapsed).toBe(true);
    expect(view.label).toBe("");
    expect(view.showFirstLaunchNote).toBe(false);
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
