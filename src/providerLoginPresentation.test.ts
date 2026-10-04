import { describe, expect, it } from "vitest";
import { plainProviderLoginOutput, providerLoginCode, providerLoginUrl } from "./providerLoginPresentation";

describe("provider login presentation", () => {
  it("extracts a safe browser handoff from colored terminal output", () => {
    const output = "\u001b[32mOpen https://auth.openai.com/oauth?code=abc-123\u001b[0m\r\n";
    expect(providerLoginUrl(output)).toBe("https://auth.openai.com/oauth?code=abc-123");
    expect(plainProviderLoginOutput(output)).toBe("Open https://auth.openai.com/oauth?code=abc-123");
  });

  it("does not promote non-http terminal text into a clickable handoff", () => {
    expect(providerLoginUrl("Open file:///tmp/token or javascript:alert(1)")).toBeNull();
  });

  it("stops OSC stripping at an ST terminator instead of eating the handoff", () => {
    const output = "\u001b]0;login\u001b\\Open https://auth.openai.com/oauth?code=abc-123\r\n";
    expect(plainProviderLoginOutput(output)).toBe("Open https://auth.openai.com/oauth?code=abc-123");
    expect(providerLoginUrl(output)).toBe("https://auth.openai.com/oauth?code=abc-123");
  });

  it("removes control noise but preserves the vendor's readable instructions", () => {
    expect(plainProviderLoginOutput("\u001b]0;login\u0007Enter code:\tABCD\r\n"))
      .toBe("Enter code:\tABCD");
  });

  it("surfaces gh's one-time device code and URL", () => {
    const output = "\u001b[0;33m!\u001b[0m One-time code (\u001b[0;1;39mEFE0-E851\u001b[0m) copied to clipboard\r\n\u001b[0;1;39mOpen this URL\u001b[0m to continue in your web browser: https://github.com/login/device\r\n";
    expect(providerLoginCode(output)).toBe("EFE0-E851");
    expect(providerLoginUrl(output)).toBe("https://github.com/login/device");
    expect(providerLoginCode("Waiting for the browser…")).toBeNull();
  });
});
