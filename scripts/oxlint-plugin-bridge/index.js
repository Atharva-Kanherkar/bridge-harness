// Bridge's own design rules for oxlint. They cover what @shadcn/lint does not:
// feature code reaching past the primitives in src/components/ui.
//
// no-native-title-tooltip follows the rule of the same name in T3 Code
// (MIT, see THIRD_PARTY_NOTICES.md).

const INTRINSIC = /^[a-z]/u;

// On these elements `title` names the embedded content for assistive tech and
// is not a hover tooltip, so the Tooltip primitive is not a replacement.
const TITLE_IS_AN_ACCESSIBLE_NAME = new Set(["embed", "frame", "iframe", "math", "object"]);

const RAW_CONTROLS = {
  button: "Button from @/components/ui/button (or a row/menu primitive that renders one)",
  input: "Input from @/components/ui/input, or Checkbox / Switch for a toggle",
  select: "Select from @/components/ui/select",
  textarea: "Textarea from @/components/ui/textarea",
};

function attribute(node, name) {
  for (const candidate of node.attributes) {
    if (candidate.type !== "JSXAttribute") continue;
    if (candidate.name.type !== "JSXIdentifier") continue;
    if (candidate.name.name === name) return candidate;
  }
  return undefined;
}

function literalValue(attr) {
  const value = attr?.value;
  if (!value) return undefined;
  if (value.type === "Literal") return value.value;
  if (value.type === "JSXExpressionContainer" && value.expression.type === "Literal") {
    return value.expression.value;
  }
  return undefined;
}

const noNativeTitleTooltip = {
  meta: {
    type: "problem",
    docs: {
      description: "Disallow the native title attribute as a tooltip; use the Tooltip primitive.",
    },
  },
  create(context) {
    return {
      JSXOpeningElement(node) {
        if (node.name.type !== "JSXIdentifier") return;
        if (!INTRINSIC.test(node.name.name)) return;
        if (TITLE_IS_AN_ACCESSIBLE_NAME.has(node.name.name)) return;
        const title = attribute(node, "title");
        if (!title) return;
        context.report({
          node: title,
          message:
            "Do not use the native title attribute as a tooltip: it is slow, unstyled and ignores the theme. Wrap the element in <Tooltip label=...> from @/components/ui/tooltip, or pass tooltip= to Button.",
        });
      },
    };
  },
};

const noRawControls = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow hand-styled native controls in feature code; components/ui owns how controls look.",
    },
  },
  create(context) {
    return {
      JSXOpeningElement(node) {
        if (node.name.type !== "JSXIdentifier") return;
        const tag = node.name.name;
        const replacement = RAW_CONTROLS[tag];
        if (!replacement) return;
        if (tag === "input") {
          // A hidden or file input has no look to own.
          const type = literalValue(attribute(node, "type"));
          if (type === "hidden" || type === "file") return;
        }
        context.report({
          node: node.name,
          message: `Do not hand-style a raw <${tag}> in feature code. Use ${replacement}. If no primitive fits, add a variant or a new primitive under src/components/ui first.`,
        });
      },
    };
  },
};

export default {
  meta: { name: "bridge" },
  rules: {
    "no-native-title-tooltip": noNativeTitleTooltip,
    "no-raw-controls": noRawControls,
  },
};
