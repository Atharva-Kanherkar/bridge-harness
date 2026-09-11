import { asWireKind } from "./transcript/wire";
import { describe, expect, it } from "vitest";
import { needsProviderSignIn, providerSignInForEvent } from "./providerLogin";

describe("provider login recovery", () => {
  it("recovers a live auth error but does not act on conversation text", () => {
    expect(providerSignInForEvent("codex", { kind: asWireKind("error"), text: "authentication required" })).toBe("codex");
    expect(providerSignInForEvent("codex", { kind: asWireKind("message.completed"), text: "authentication required" })).toBeNull();
  });
  it("starts the matching provider login for an expired session", () => {
    for (const provider of ["codex", "claude", "cursor", "opencode"])
      expect(needsProviderSignIn(provider, "Your access token has expired")).toBe(provider);
  });
  it("recovers HTTP authentication and revoked refresh-token errors", () => {
    expect(needsProviderSignIn("opencode", "HTTP 401 Unauthorized")).toBe("opencode");
    expect(needsProviderSignIn("codex", "Your refresh token was already used. Please log out and sign in again.")).toBe("codex");
    expect(needsProviderSignIn("codex", "The refresh token has been revoked")).toBe("codex");
  });
  it("does not turn policy, network, quota, or unknown-provider errors into login", () => {
    for (const message of ["403 forbidden", "429 rate limit", "connection refused", "write scope rejected"])
      expect(needsProviderSignIn("codex", message)).toBeNull();
    expect(needsProviderSignIn("external", "authentication required")).toBeNull();
  });
});
