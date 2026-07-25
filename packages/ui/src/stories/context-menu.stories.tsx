import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { ContextMenu, type ContextMenuEntry } from "../index";

const entries: ContextMenuEntry[] = [
  { id: "message", label: "Send message", onSelect: () => undefined },
  { id: "workbench", label: "Workbench", onSelect: () => undefined },
  { id: "separator", kind: "separator" },
  { id: "settings", label: "Language model settings", onSelect: () => undefined },
  { id: "avatar", label: "3D model settings", onSelect: () => undefined },
  { id: "voice", label: "Voice settings", onSelect: () => undefined },
  { id: "separator-2", kind: "separator" },
  { id: "always-on-top", label: "Always on top", checked: true, onSelect: () => undefined },
  { id: "exit", label: "Exit", onSelect: () => undefined },
];

function ContextMenuExample() {
  return (
    <ContextMenu
      entries={entries}
      trigger={
        <div
          style={{
            width: "320px",
            padding: "48px 24px",
            border: "1px dashed var(--color-border-strong)",
            "border-radius": "var(--radius-lg)",
            "text-align": "center",
          }}
        >
          Right-click this area
        </div>
      }
    />
  );
}

const meta = {
  title: "Components/ContextMenu",
  component: ContextMenuExample,
} satisfies Meta<typeof ContextMenuExample>;

export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
