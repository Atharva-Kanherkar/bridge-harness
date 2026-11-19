// One answerable shape for every harness's question.
//
// Each provider asks differently, and the card must not care which one asked:
//   - Claude `AskUserQuestion`: questions with `header`, `options`
//     ({label, description, preview}), `multiSelect`; a typed "Other" is
//     always allowed (core stamps `allowOther` and a positional `id`).
//   - OpenCode `question`: the same shape with `multiple`, and `custom`
//     (default true) for a typed answer.
//   - Codex `requestUserInput`: questions with `id`, `header`, `question`,
//     `options` or none (free text), `isOther`, `isSecret`.
//   - MCP elicitation (Codex): a JSON Schema `requestedSchema` whose
//     properties are strings, numbers, booleans, enums or arrays of enums.
//
// Answers go back keyed by field id as string arrays, which is what
// `resolve_question` takes for every provider.

export type QuestionOption = { value: string; label: string; description?: string; preview?: string };

export type QuestionField = {
  id: string;
  /** A short chip above the prompt ("Database", "Auth method"). */
  header?: string;
  prompt: string;
  /** Extra explanation shown under the prompt. */
  hint?: string;
  kind: "choice" | "text" | "number" | "boolean";
  options: QuestionOption[];
  multiple: boolean;
  /** A choice field that also takes a typed answer. */
  allowOther: boolean;
  secret: boolean;
  required: boolean;
  integer: boolean;
  defaultValue: string[];
};

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json => !!value && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): string | undefined => typeof value === "string" && value.trim() ? value.trim() : undefined;

function readOption(value: unknown): QuestionOption | null {
  if (typeof value === "string") return value.trim() ? { value, label: value } : null;
  if (typeof value === "number" || typeof value === "boolean") return { value: String(value), label: String(value) };
  if (!isObject(value)) return null;
  const label = text(value.label) ?? text(value.title) ?? text(value.name) ?? (value.const !== undefined ? String(value.const) : undefined) ?? text(value.value);
  if (!label) return null;
  const raw = value.const ?? value.value;
  return {
    value: raw !== undefined && raw !== null && typeof raw !== "object" ? String(raw) : label,
    label,
    description: text(value.description),
    preview: typeof value.preview === "string" && value.preview.trim() ? value.preview : undefined,
  };
}

function questionListFields(questions: unknown[], typedByDefault: boolean): QuestionField[] {
  return questions.flatMap((value, index): QuestionField[] => {
    if (!isObject(value)) return [];
    const options = Array.isArray(value.options) ? value.options.map(readOption).filter((option): option is QuestionOption => option !== null) : [];
    const header = text(value.header);
    const prompt = text(value.question) ?? header ?? text(value.label) ?? "Answer";
    // Claude stamps `allowOther`; OpenCode's `custom` defaults on; Codex says `isOther`.
    const allowOther = value.allowOther === true || value.isOther === true || (value.custom !== undefined ? value.custom !== false : typedByDefault);
    return [{
      id: text(value.id) ?? String(index),
      header: header && header !== prompt ? header : undefined,
      prompt,
      hint: text(value.description),
      kind: options.length ? "choice" : "text",
      options,
      multiple: value.multiSelect === true || value.multiple === true,
      allowOther: options.length ? allowOther : false,
      secret: value.isSecret === true,
      required: true,
      integer: false,
      defaultValue: [],
    }];
  });
}

function schemaOptions(property: Json): QuestionOption[] {
  if (Array.isArray(property.enum)) {
    const names = Array.isArray(property.enumNames) ? property.enumNames : [];
    return property.enum.flatMap((value, index) => {
      const option = readOption(value);
      if (!option) return [];
      const label = text(names[index]);
      return [label ? { ...option, label } : option];
    });
  }
  const variants = Array.isArray(property.oneOf) ? property.oneOf : Array.isArray(property.anyOf) ? property.anyOf : [];
  return variants.map(readOption).filter((option): option is QuestionOption => option !== null);
}

function schemaDefault(value: unknown): string[] {
  if (Array.isArray(value)) return value.filter(item => item !== null && typeof item !== "object").map(String);
  if (value === undefined || value === null || typeof value === "object") return [];
  return [String(value)];
}

function schemaFields(schema: Json): QuestionField[] {
  const properties = isObject(schema.properties) ? schema.properties : {};
  const required = new Set(Array.isArray(schema.required) ? schema.required.map(String) : []);
  return Object.entries(properties).map(([id, value]): QuestionField => {
    const property = isObject(value) ? value : {};
    const items = isObject(property.items) ? property.items : {};
    const multiple = property.type === "array";
    const options = schemaOptions(multiple ? items : property);
    const title = text(property.title);
    const description = text(property.description);
    const kind: QuestionField["kind"] = options.length
      ? "choice"
      : property.type === "boolean"
        ? "boolean"
        : property.type === "number" || property.type === "integer"
          ? "number"
          : "text";
    return {
      id,
      prompt: title ?? description ?? id,
      hint: title && description ? description : undefined,
      kind,
      options: kind === "boolean" ? [{ value: "true", label: "Yes" }, { value: "false", label: "No" }] : options,
      multiple,
      allowOther: false,
      secret: property.format === "password",
      required: required.has(id),
      integer: property.type === "integer",
      defaultValue: schemaDefault(property.default),
    };
  });
}

/** The fields a pending question asks, in the order the provider asked them. */
export function questionFields(data: Json): QuestionField[] {
  // OpenCode's `custom` is documented as on unless the question turns it off.
  if (Array.isArray(data.questions) && data.questions.length) return questionListFields(data.questions, data.requestMethod === "opencode.question");
  if (isObject(data.requestedSchema)) return schemaFields(data.requestedSchema);
  return [];
}

/** Whether one field's answer is complete enough to send. */
export function fieldAnswered(field: QuestionField, values: readonly string[] | undefined): boolean {
  const filled = (values ?? []).filter(value => value.trim());
  if (!filled.length) return false;
  if (field.kind === "number") return filled.every(value => Number.isFinite(Number(value)) && (!field.integer || Number.isInteger(Number(value))));
  return true;
}

/** Whether the whole set can be sent: every required field answered, and at
 *  least one answer when every field is optional. */
export function canSubmit(fields: readonly QuestionField[], answers: Record<string, string[]>): boolean {
  if (!fields.length) return false;
  const anyAnswered = fields.some(field => fieldAnswered(field, answers[field.id]));
  return anyAnswered && fields.every(field => !field.required || fieldAnswered(field, answers[field.id]));
}

/** The starting answers: a schema's defaults, nothing else. */
export function initialAnswers(fields: readonly QuestionField[]): Record<string, string[]> {
  return Object.fromEntries(fields.filter(field => field.defaultValue.length).map(field => [field.id, field.defaultValue]));
}

/** The answers as they will be sent: trimmed, empties dropped. */
export function cleanAnswers(fields: readonly QuestionField[], answers: Record<string, string[]>): Record<string, string[]> {
  return Object.fromEntries(fields.flatMap(field => {
    const values = (answers[field.id] ?? []).map(value => value.trim()).filter(Boolean);
    return values.length ? [[field.id, values]] : [];
  }));
}

/** A settled answer as words: option labels where they exist, never a secret. */
export function answerSummary(field: QuestionField, values: readonly string[] | undefined): string | null {
  if (field.secret) return values?.length ? "Hidden" : null;
  const shown = (values ?? []).map(value => field.options.find(option => option.value === value)?.label ?? value).filter(Boolean);
  return shown.length ? shown.join(", ") : null;
}
