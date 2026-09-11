import { describe, expect, it } from "vitest";
import { plainProviderLoginOutput, providerLoginUrl } from "./providerLoginPresentation";

describe("provider login presentation", () => {
  it("extracts a safe browser handoff from colored terminal output", () => {
    const output = "\u001b[32mOpen https://auth.openai.com/oauth?code=abc-123\u001b[0m\r\n";
    expect(providerLoginUrl(output)).toBe("https://auth.openai.com/oauth?code=abc-123");
    expect(plainProviderLoginOutput(output)).toBe("Open https://auth.openai.com/oauth?code=abc-123");
  });

  it("does not promote non-http terminal text into a clickable handoff", () => {
    expect(providerLoginUrl("Open file:///tmp/token or javascript:alert(1)")).toBeNull();
  });

  it("removes control noise but preserves the vendor's readable instructions", () => {
    expect(plainProviderLoginOutput("\u001b]0;login\u0007Enter code:\tABCD\r\n"))
      .toBe("Enter code:\tABCD");
  });
});
