import eslint from "@eslint/js";
import globals from "globals";
import solid from "eslint-plugin-solid/configs/typescript";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: [
      "**/dist/**",
      "**/storybook-static/**",
      "**/generated.ts",
      "**/target/**",
      "**/target-*/**",
    ],
  },
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  solid,
  {
    files: ["scripts/**/*.mjs"],
    languageOptions: { globals: globals.node },
  },
  {
    files: ["scripts/desktop-e2e/**/*.mjs"],
    languageOptions: {
      globals: {
        ...globals.node,
        ...globals.mocha,
        browser: "readonly",
        $: "readonly",
        $$: "readonly",
        expect: "readonly",
      },
    },
  },
  {
    files: ["docs/ui-style-demos/**/*.js"],
    languageOptions: { globals: globals.browser },
  },
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
    rules: {
      "@typescript-eslint/consistent-type-imports": "error",
      "@typescript-eslint/no-explicit-any": "off",
    },
  },
);
