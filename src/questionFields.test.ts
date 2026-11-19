import { describe, expect, it } from "vitest";
import { answerSummary, canSubmit, cleanAnswers, initialAnswers, questionFields } from "./questionFields";

describe("questionFields", () => {
  it("reads Claude's AskUserQuestion with descriptions, previews and a typed Other", () => {
    const fields = questionFields({ questions: [
      { id: "0", question: "Which database?", header: "Database", multiSelect: false, allowOther: true,
        options: [{ label: "Postgres", description: "Relational" }, { label: "SQLite", description: "Embedded", preview: "app.db" }] },
      { id: "1", question: "Which features?", header: "Features", multiSelect: true, allowOther: true,
        options: [{ label: "Auth", description: "" }, { label: "Billing", description: "" }] },
    ] });
    expect(fields).toHaveLength(2);
    expect(fields[0]).toMatchObject({ id: "0", header: "Database", prompt: "Which database?", kind: "choice", multiple: false, allowOther: true });
    expect(fields[0].options[1]).toEqual({ value: "SQLite", label: "SQLite", description: "Embedded", preview: "app.db" });
    expect(fields[1].multiple).toBe(true);
  });

  it("reads OpenCode's custom flag (on by default) and multiple", () => {
    const [open, closed] = questionFields({ requestMethod: "opencode.question", questions: [
      { question: "Pick", options: [{ label: "A" }], multiple: true },
      { question: "Strict", options: [{ label: "B" }], custom: false },
    ] });
    expect(open).toMatchObject({ id: "0", multiple: true, allowOther: true });
    expect(closed).toMatchObject({ id: "1", allowOther: false });
    expect(questionFields({ questions: [{ question: "Q", options: [{ label: "A" }] }] })[0].allowOther).toBe(false);
  });

  it("reads Codex free-text, secret and isOther questions", () => {
    const fields = questionFields({ questions: [
      { id: "name", header: "Name", question: "What should it be called?", options: null },
      { id: "token", question: "Paste the token", isSecret: true },
      { id: "env", question: "Where?", isOther: true, options: [{ label: "staging", description: "" }] },
    ] });
    expect(fields.map(field => field.kind)).toEqual(["text", "text", "choice"]);
    expect(fields[1].secret).toBe(true);
    expect(fields[2].allowOther).toBe(true);
  });

  it("reads an MCP elicitation schema: enums, booleans, numbers, arrays, defaults, required", () => {
    const fields = questionFields({ message: "Configure", requestedSchema: { type: "object", required: ["env"], properties: {
      env: { type: "string", title: "Environment", enum: ["dev", "prod"], enumNames: ["Development", "Production"] },
      confirm: { type: "boolean", title: "Proceed?", default: true },
      count: { type: "integer", title: "Replicas" },
      tags: { type: "array", title: "Tags", items: { anyOf: [{ const: "a", title: "Alpha" }, { const: "b", title: "Beta" }] } },
      pw: { type: "string", format: "password", description: "Password" },
    } } });
    const byId = Object.fromEntries(fields.map(field => [field.id, field]));
    expect(byId.env).toMatchObject({ kind: "choice", required: true, prompt: "Environment" });
    expect(byId.env.options.map(option => option.label)).toEqual(["Development", "Production"]);
    expect(byId.confirm).toMatchObject({ kind: "boolean", required: false, defaultValue: ["true"] });
    expect(byId.count).toMatchObject({ kind: "number", integer: true });
    expect(byId.tags).toMatchObject({ kind: "choice", multiple: true });
    expect(byId.tags.options.map(option => option.value)).toEqual(["a", "b"]);
    expect(byId.pw).toMatchObject({ secret: true, prompt: "Password" });
    expect(initialAnswers(fields)).toEqual({ confirm: ["true"] });
  });

  it("asks for every required answer, numbers that parse, and at least one answer", () => {
    const fields = questionFields({ requestedSchema: { required: ["n"], properties: { n: { type: "integer" }, note: { type: "string" } } } });
    expect(canSubmit(fields, {})).toBe(false);
    expect(canSubmit(fields, { n: ["1.5"] })).toBe(false);
    expect(canSubmit(fields, { n: ["2"] })).toBe(true);
    expect(canSubmit([], {})).toBe(false);
    expect(cleanAnswers(fields, { n: [" 2 "], note: ["  "] })).toEqual({ n: ["2"] });
  });

  it("summarises an answer by label and never shows a secret", () => {
    const [choice, secret] = questionFields({ questions: [
      { question: "Env", options: [{ label: "Production", value: "prod" }] },
      { question: "Token", isSecret: true },
    ] });
    expect(answerSummary(choice, ["prod", "typed"])).toBe("Production, typed");
    expect(answerSummary(secret, ["sk"])).toBe("Hidden");
    expect(answerSummary(choice, [])).toBeNull();
  });

  it("returns nothing for a shape it cannot answer", () => {
    expect(questionFields({})).toEqual([]);
  });
});
