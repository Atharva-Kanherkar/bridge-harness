import { describe, expect, it } from "vitest";
import type { ConversationItem } from "./conversation";
import { failedTurn } from "./failedTurn";

let sequence = 0;
const row = (type: ConversationItem["type"], extra: Partial<ConversationItem> = {}): ConversationItem => {
  sequence += 1;
  return { key: `${type}-${sequence}`, type, eventId: sequence, text: "", data: {}, sequence, turn: 1, ...extra };
};
const user = (text: string) => row("message", { role: "user", text });
const reply = (text: string) => row("message", { role: "assistant", text });
const error = (text = "Insufficient credits", status?: string) => row("error", { text, status });

describe("failedTurn", () => {
  it("finds the error the last turn ended on and the message it answers", () => {
    const prompt = user("/rimo-bug-hunt pls use it now.");
    const failure = error();
    expect(failedTurn([user("hi"), reply("hello"), prompt, failure])).toEqual({ errorKey: failure.key, prompt });
  });

  it("counts tool rows, thinking and partial text as part of the failed turn", () => {
    const prompt = user("fix the lint");
    const failure = error("rate limited");
    const items = [prompt, row("reasoning"), row("activity"), reply("Looking at"), failure, row("raw")];
    expect(failedTurn(items)).toEqual({ errorKey: failure.key, prompt });
  });

  it("offers nothing once a reply follows the error", () => {
    expect(failedTurn([user("go"), error(), reply("recovered")])).toBeNull();
  });

  it("offers nothing once a newer message follows the error", () => {
    expect(failedTurn([user("go"), error(), user("go again")])).toBeNull();
  });

  it("offers nothing for a turn that ended well", () => {
    expect(failedTurn([user("go"), reply("done")])).toBeNull();
    expect(failedTurn([])).toBeNull();
  });

  it("does not treat a degraded history entry as a failed turn", () => {
    expect(failedTurn([user("go"), reply("done"), error("missing", "degraded")])).toBeNull();
    const prompt = user("go");
    const failure = error();
    expect(failedTurn([prompt, failure, error("missing", "degraded")])).toEqual({ errorKey: failure.key, prompt });
  });

  it("offers nothing when no message came before the error", () => {
    expect(failedTurn([row("activity"), error()])).toBeNull();
  });
});
