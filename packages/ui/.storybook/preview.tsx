import type { Preview } from "storybook-solidjs-vite";
import "../src/styles/index.css";

const preview: Preview = {
  globalTypes: {
    theme: {
      description: "Color scheme",
      defaultValue: "light",
      toolbar: {
        icon: "paintbrush",
        items: ["light", "dark", "system"],
      },
    },
    locale: {
      description: "Locale",
      defaultValue: "zh-CN",
      toolbar: {
        icon: "globe",
        items: ["zh-CN", "en-US"],
      },
    },
    zoom: {
      description: "UI zoom",
      defaultValue: "100",
      toolbar: {
        icon: "zoom",
        items: ["100", "125", "150"],
      },
    },
  },
  decorators: [
    (Story, context) => {
      const requestedTheme = String(context.globals.theme ?? "light");
      const theme =
        requestedTheme === "system"
          ? window.matchMedia("(prefers-color-scheme: dark)").matches
            ? "dark"
            : "light"
          : requestedTheme === "dark"
            ? "dark"
            : "light";
      document.documentElement.dataset.colorScheme = theme;
      document.documentElement.lang = context.globals.locale === "en-US" ? "en-US" : "zh-CN";
      const zoom = Number(context.globals.zoom ?? 100) / 100;
      return (
        <div
          style={{
            "min-height": "100vh",
            padding: "24px",
            background: "var(--color-bg-base)",
            color: "var(--color-text-base)",
            zoom,
          }}
        >
          <Story />
        </div>
      );
    },
  ],
  parameters: {
    controls: { expanded: true },
    a11y: { test: "error" },
    backgrounds: { disable: true },
  },
};

export default preview;
