import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { For } from "solid-js";

function Tokens() {
  const colors = [
    "--color-bg-base",
    "--color-bg-subtle",
    "--color-bg-elevated",
    "--color-bg-accent",
    "--color-text-base",
    "--color-text-muted",
    "--color-state-success",
    "--color-state-warning",
    "--color-state-danger",
  ];
  return (
    <div>
      <h1>Semantic tokens</h1>
      <div style={{ display: "grid", "grid-template-columns": "repeat(3, 1fr)", gap: "12px" }}>
        <For each={colors}>
          {(color) => (
            <div
              style={{
                border: "1px solid var(--color-border-base)",
                "border-radius": "8px",
                padding: "12px",
              }}
            >
              <div
                style={{ height: "64px", background: `var(${color})`, "border-radius": "6px" }}
              />
              <code>{color}</code>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}

const meta = { title: "Foundations/Tokens", component: Tokens } satisfies Meta<typeof Tokens>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
