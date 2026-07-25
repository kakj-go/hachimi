import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";
import {
  ColorField,
  RangeField,
  SegmentedControl,
  SelectField,
  SettingsRow,
  SettingsSection,
  Switch,
  TextField,
  Toast,
} from "../index";

const meta = {
  title: "Components/Forms",
  component: TextField,
} satisfies Meta<typeof TextField>;

export default meta;
type Story = StoryObj<typeof meta>;

export const TextInput: Story = {
  args: {
    label: "API Base URL",
    placeholder: "http://localhost:11434/v1",
    description: "Phase 4 placeholder",
  },
};

export const SettingsSwitch: Story = {
  args: { label: "Always on top" },
  render: () => {
    const [checked, setChecked] = createSignal(true);
    return (
      <SettingsSection title="Appearance">
        <SettingsRow label="Always on top" description="Keep the pet above other windows">
          <Switch checked={checked()} onChange={setChecked} label="Always on top" />
        </SettingsRow>
      </SettingsSection>
    );
  },
};

export const AppearanceControls: Story = {
  args: { label: "Appearance controls" },
  render: () => {
    const [profile, setProfile] = createSignal("codex-dark");
    const [accent, setAccent] = createSignal("#2EA8FF");
    const [contrast, setContrast] = createSignal(60);
    const [markers, setMarkers] = createSignal<"color" | "signs">("color");
    const [toastOpen, setToastOpen] = createSignal(true);
    return (
      <div style={{ display: "grid", gap: "18px", width: "520px" }}>
        <SelectField
          label="Theme profile"
          value={profile()}
          options={[
            { value: "codex-light", label: "Codex Light" },
            { value: "codex-dark", label: "Codex Dark" },
          ]}
          onChange={setProfile}
        />
        <ColorField label="Accent" value={accent()} onInput={setAccent} />
        <RangeField
          label="Contrast"
          value={contrast()}
          min={0}
          max={100}
          unit="%"
          onInput={setContrast}
        />
        <SegmentedControl
          label="Diff markers"
          value={markers()}
          options={[
            { value: "color", label: "Color" },
            { value: "signs", label: "Signs" },
          ]}
          onChange={setMarkers}
        />
        <Toast open={toastOpen()} onClose={() => setToastOpen(false)}>
          Appearance saved
        </Toast>
      </div>
    );
  },
};
