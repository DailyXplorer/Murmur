// Text buttons carry text only: a <Button> may hold icons alone, or text
// alone, never an icon leading its text. The meeting view's Back arrow is the
// one disabled exception.

const elementName = (node) => {
  const name = node.openingElement.name;
  if (name.type === "JSXIdentifier") return name.name;
  if (name.type === "JSXMemberExpression") return name.property.name;
  return "";
};

const isBlank = (child) =>
  (child.type === "JSXText" && child.value.trim() === "") ||
  (child.type === "JSXExpressionContainer" &&
    child.expression.type === "JSXEmptyExpression");

const isIcon = (child) =>
  child.type === "JSXElement" && elementName(child).endsWith("Icon");

export default {
  meta: {
    type: "problem",
    messages: {
      iconInTextButton:
        "Text buttons carry no icon: remove the icon, or keep the icon alone with an aria-label.",
    },
    schema: [],
  },
  create(context) {
    return {
      JSXElement(node) {
        if (elementName(node) !== "Button") return;
        const [first, ...rest] = node.children.filter(
          (child) => !isBlank(child),
        );
        if (first && isIcon(first) && rest.some((child) => !isIcon(child))) {
          context.report({ node: first, messageId: "iconInTextButton" });
        }
      },
    };
  },
};
