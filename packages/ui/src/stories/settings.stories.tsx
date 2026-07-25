import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";
import { Badge, NativeSelect, SettingsRow, SettingsSection, Switch, Tabs } from "../index";

function SettingsExample(props: { locale: "zh-CN" | "en-US" }) {
  const [tab, setTab] = createSignal("general");
  const [top, setTop] = createSignal(true);
  const zh = () => props.locale === "zh-CN";
  return (
    <main style={{ width: "760px", margin: "0 auto" }}>
      <header
        style={{ display: "flex", "justify-content": "space-between", "align-items": "center" }}
      >
        <h1>{zh() ? "设置" : "Settings"}</h1>
        <Badge>{zh() ? "Windows 先行" : "Windows first"}</Badge>
      </header>
      <Tabs
        value={tab()}
        onChange={setTab}
        tabs={[
          {
            value: "general",
            label: zh() ? "通用" : "General",
            content: (
              <SettingsSection title={zh() ? "外观" : "Appearance"}>
                <SettingsRow label={zh() ? "始终置顶" : "Always on top"}>
                  <Switch checked={top()} onChange={setTop} label="Always on top" />
                </SettingsRow>
                <SettingsRow label={zh() ? "主题" : "Theme"}>
                  <NativeSelect label={zh() ? "主题" : "Theme"} aria-label="Theme">
                    <option>{zh() ? "跟随系统" : "Use system"}</option>
                    <option>{zh() ? "浅色" : "Light"}</option>
                    <option>{zh() ? "深色" : "Dark"}</option>
                  </NativeSelect>
                </SettingsRow>
              </SettingsSection>
            ),
          },
          { value: "llm", label: zh() ? "大语言模型" : "Language model", content: <p>Phase 4</p> },
          { value: "avatar", label: zh() ? "3D 模型" : "3D model", content: <p>Phase 2</p> },
          { value: "voice", label: zh() ? "语音" : "Voice", content: <p>Phase 6</p> },
        ]}
      />
    </main>
  );
}

const meta = {
  title: "Examples/Settings",
  component: SettingsExample,
  render: (_args, context) => (
    <SettingsExample locale={context.globals.locale === "en-US" ? "en-US" : "zh-CN"} />
  ),
} satisfies Meta<typeof SettingsExample>;

export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = { args: { locale: "zh-CN" } };
