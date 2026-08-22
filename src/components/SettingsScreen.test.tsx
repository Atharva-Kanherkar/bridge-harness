import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { SettingsScreen } from "./SettingsScreen";

describe("SettingsScreen", () => {
  it("exposes the configuration areas before asynchronous data loads", () => {
    const html = renderToStaticMarkup(<SettingsScreen adapters={[]} onModelSetupChange={() => undefined} onSuggestionSettingsChange={() => undefined} onError={() => undefined} />);
    expect(html).toContain("Settings");
    expect(html).toContain("Agents");
    expect(html).toContain("Harnesses");
    expect(html).toContain("Role models");
    expect(html).toContain("Prompts change behavior, never permissions");
    expect(html).toContain("Permissions");
  });
});
