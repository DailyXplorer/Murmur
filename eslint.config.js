import i18next from "eslint-plugin-i18next";
import tsParser from "@typescript-eslint/parser";
import noIconInTextButton from "./eslint-rules/no-icon-in-text-button.js";

export default [
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaFeatures: {
          jsx: true,
        },
      },
    },
    plugins: {
      i18next,
      murmur: { rules: { "no-icon-in-text-button": noIconInTextButton } },
    },
    rules: {
      "murmur/no-icon-in-text-button": "error",
      // Catch text in JSX that should be translated
      "i18next/no-literal-string": [
        "error",
        {
          markupOnly: true, // Only check JSX content, not all strings
          ignoreAttribute: [
            "className",
            "style",
            "type",
            "id",
            "name",
            "key",
            "data-*",
            "aria-*",
          ], // Ignore common non-translatable attributes
        },
      ],
    },
  },
];
