import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { For, Show } from "solid-js";
import {
  Badge,
  ArrowLeft,
  ArrowRight,
  Button,
  FolderOpen,
  Monitor,
  Moon,
  NativeSelect,
  NumberField,
  ResourceCard,
  ResourceList,
  RangeField,
  SearchField,
  SelectField,
  Settings,
  SettingsRow,
  SettingsSection,
  SidebarNav,
  StatusBanner,
  TextField,
  ThemeCard,
  TitleBar,
  Sun,
  Switch,
} from "../index";

type PreviewRoute = "home" | "general" | "appearance" | "llm" | "avatar" | "voice";

function WorkbenchPreview(props: { route: PreviewRoute; locale: "zh-CN" | "en-US" }) {
  const zh = () => props.locale === "zh-CN";
  const labels = () => [
    { value: "general" as const, label: zh() ? "通用" : "General" },
    { value: "appearance" as const, label: zh() ? "外观" : "Appearance" },
    { value: "llm" as const, label: zh() ? "大语言模型" : "Language model" },
    { value: "avatar" as const, label: zh() ? "3D 模型" : "3D models" },
    { value: "voice" as const, label: zh() ? "语音" : "Voice" },
  ];
  return (
    <div
      style={{
        width: "1180px",
        height: "720px",
        border: "1px solid var(--color-border-base)",
        overflow: "hidden",
        background: "var(--color-bg-base)",
      }}
    >
      <TitleBar brand="Hachimi">
        <span style={{ color: "var(--color-text-faint)", display: "flex", gap: "9px" }}>
          <ArrowLeft size={15} /> <ArrowRight size={15} />
          <span>{zh() ? "文件 / 编辑 / 视图 / 帮助" : "File / Edit / View / Help"}</span>
        </span>
      </TitleBar>
      <div style={{ display: "grid", "grid-template-columns": "270px 1fr", height: "676px" }}>
        <aside
          style={{
            padding: "20px 14px",
            border: "0",
            "border-right": "1px solid var(--color-border-muted)",
            background: "var(--color-bg-subtle)",
            display: "flex",
            "flex-direction": "column",
          }}
        >
          <SearchField label="Search" placeholder={zh() ? "搜索设置…" : "Search settings…"} />
          <p
            style={{ color: "var(--color-text-faint)", "font-size": "12px", "margin-top": "24px" }}
          >
            HACHIMI DESKTOP
          </p>
          <Show
            when={props.route !== "home"}
            fallback={
              <>
                <strong style={{ display: "flex", gap: "7px", "align-items": "center" }}>
                  <FolderOpen size={15} /> Hachimi
                </strong>
                <p>{zh() ? "设计桌宠工作台和设置中心" : "Design the Pet workbench"}</p>
                <p>{zh() ? "讨论跨平台技术选型" : "Cross-platform architecture"}</p>
              </>
            }
          >
            <SidebarNav
              label="Settings"
              items={labels()}
              value={props.route as Exclude<PreviewRoute, "home">}
              onChange={() => undefined}
            />
          </Show>
          <Show when={props.route === "home"}>
            <button
              type="button"
              aria-label={zh() ? "设置" : "Settings"}
              style={{
                "margin-top": "auto",
                display: "flex",
                "align-items": "center",
                gap: "9px",
                height: "38px",
                border: "0",
                background: "transparent",
                color: "var(--color-text-muted)",
              }}
            >
              <Settings size={17} /> {zh() ? "设置" : "Settings"}
            </button>
          </Show>
        </aside>
        <main style={{ padding: "52px 68px", overflow: "hidden" }}>
          <Show
            when={props.route !== "home"}
            fallback={
              <div
                style={{
                  display: "grid",
                  "place-items": "center",
                  height: "100%",
                  "text-align": "center",
                }}
              >
                <div>
                  <Badge>Hachimi Workbench</Badge>
                  <h1 style={{ "font-size": "34px" }}>
                    {zh()
                      ? "今天想和 Hachimi 一起做些什么？"
                      : "What should we do with Hachimi today?"}
                  </h1>
                  <div
                    style={{
                      display: "grid",
                      "grid-template-columns": "repeat(2, 240px)",
                      gap: "12px",
                      "margin-top": "30px",
                    }}
                  >
                    <For each={labels()}>
                      {(item) => <Button size="large">{item.label}</Button>}
                    </For>
                  </div>
                </div>
              </div>
            }
          >
            <h1>{labels().find((item) => item.value === props.route)?.label}</h1>
            <p style={{ color: "var(--color-text-faint)" }}>
              {zh()
                ? "所有设置都保存在本机，并遵循最小权限边界。"
                : "Settings stay local and follow least-privilege boundaries."}
            </p>
            <div style={{ display: "grid", gap: "16px", "margin-top": "32px" }}>
              <Show when={props.route === "general"}>
                <StatusBanner>
                  {zh() ? "主题、语言与桌宠始终置顶" : "Theme, language, and Pet always-on-top"}
                </StatusBanner>
              </Show>
              <Show when={props.route === "appearance"}>
                <div
                  style={{
                    display: "grid",
                    "grid-template-columns": "repeat(3, 1fr)",
                    gap: "12px",
                  }}
                >
                  <ThemeCard
                    label={zh() ? "跟随系统" : "System"}
                    selected
                    preview={<Monitor size={54} />}
                  />
                  <ThemeCard label={zh() ? "浅色" : "Light"} preview={<Sun size={54} />} />
                  <ThemeCard label={zh() ? "深色" : "Dark"} preview={<Moon size={54} />} />
                </div>
              </Show>
              <Show when={props.route === "llm"}>
                <TextField
                  label={zh() ? "接口地址" : "Endpoint URL"}
                  value="http://localhost:11434/v1"
                />
                <TextField label={zh() ? "模型名称" : "Model name"} value="gpt-5.6-sol" />
                <div style={{ display: "grid", "grid-template-columns": "1fr 1fr", gap: "16px" }}>
                  <NumberField
                    label={zh() ? "最大输入 Token" : "Max input tokens"}
                    value={1_050_000}
                  />
                  <NumberField
                    label={zh() ? "最大输出 Token" : "Max output tokens"}
                    value={128_000}
                  />
                </div>
              </Show>
              <Show when={props.route === "voice"}>
                <StatusBanner tone="success">
                  {zh()
                    ? "SenseVoice-Small 与原生 VITS 均可用"
                    : "SenseVoice-Small and native VITS are ready"}
                </StatusBanner>
                <div style={{ display: "grid", "grid-template-columns": "1fr 180px", gap: "16px" }}>
                  <NativeSelect label={zh() ? "VITS 语音模型" : "VITS voice model"} value="local">
                    <option value="local">Hachimi 中文女声 · reference.wav</option>
                  </NativeSelect>
                  <NumberField
                    label={zh() ? "语速（百分比）" : "Speed (percent)"}
                    value={100}
                    min={50}
                    max={200}
                  />
                </div>
              </Show>
              <Show when={props.route === "avatar"}>
                <SettingsSection title={zh() ? "角色行为" : "Character behavior"}>
                  <SettingsRow
                    label={zh() ? "启用标准待机" : "Enable standard idle"}
                    description={
                      zh()
                        ? "持续运行自然姿势与随机待机场景"
                        : "Continuously run relaxed poses and random idle scenes"
                    }
                  >
                    <Switch checked label={zh() ? "启用标准待机" : "Enable standard idle"} />
                  </SettingsRow>
                  <SettingsRow label={zh() ? "活跃程度" : "Activity level"}>
                    <SelectField
                      label={zh() ? "活跃程度" : "Activity level"}
                      value="natural"
                      options={[
                        { value: "quiet", label: zh() ? "安静" : "Quiet" },
                        { value: "natural", label: zh() ? "自然" : "Natural" },
                        { value: "lively", label: zh() ? "活跃" : "Lively" },
                      ]}
                    />
                  </SettingsRow>
                  <SettingsRow label={zh() ? "动作强度" : "Motion intensity"}>
                    <RangeField
                      label={zh() ? "动作强度" : "Motion intensity"}
                      min={25}
                      max={100}
                      unit="%"
                      value={100}
                    />
                  </SettingsRow>
                </SettingsSection>
                <ResourceList label="Resources">
                  <ResourceCard
                    title="Mimi"
                    subtitle="mimi.vrm · 42.8 MB"
                    current
                    meta={
                      <div style={{ display: "flex", gap: "6px" }}>
                        <Badge>VRM 0.x</Badge>
                        <Badge tone="success">Runtime Ready</Badge>
                      </div>
                    }
                    actions={<Badge tone="success">{zh() ? "当前使用" : "Current"}</Badge>}
                  />
                  <ResourceCard
                    title={zh() ? "Mimi 夏日版" : "Mimi Summer"}
                    subtitle="summer.vrm · 48.1 MB"
                    meta={
                      <div style={{ display: "flex", gap: "6px" }}>
                        <Badge>VRM 1.0</Badge>
                        <Badge tone="success">Runtime Ready</Badge>
                      </div>
                    }
                    actions={<Button size="small">{zh() ? "设为当前" : "Set current"}</Button>}
                  />
                  <ResourceCard
                    title={zh() ? "能力不完整的 VRM" : "Incomplete VRM"}
                    subtitle="incomplete.vrm · 18.2 MB"
                    tone="danger"
                    meta={
                      <div style={{ display: "flex", gap: "6px" }}>
                        <Badge>VRM 1.0</Badge>
                        <Badge tone="danger">
                          {zh() ? "缺少运行能力" : "Missing runtime capability"}
                        </Badge>
                      </div>
                    }
                    actions={
                      <Button size="small" variant="danger">
                        {zh() ? "删除" : "Delete"}
                      </Button>
                    }
                  />
                  <ResourceCard
                    title={zh() ? "旧模型（已隔离）" : "Legacy model (quarantined)"}
                    subtitle="legacy.glb · 9.7 MB"
                    tone="danger"
                    meta={
                      <Badge tone="neutral">{zh() ? "旧版不再支持" : "Legacy unsupported"}</Badge>
                    }
                    actions={
                      <Button size="small" variant="danger">
                        {zh() ? "删除" : "Delete"}
                      </Button>
                    }
                  />
                </ResourceList>
              </Show>
            </div>
          </Show>
        </main>
      </div>
    </div>
  );
}

const meta = {
  title: "Examples/Workbench",
  component: WorkbenchPreview,
  render: (args, context) => (
    <WorkbenchPreview {...args} locale={context.globals.locale === "en-US" ? "en-US" : "zh-CN"} />
  ),
} satisfies Meta<typeof WorkbenchPreview>;

export default meta;
type Story = StoryObj<typeof meta>;
export const Home: Story = { args: { route: "home", locale: "zh-CN" } };
export const General: Story = { args: { route: "general", locale: "zh-CN" } };
export const Appearance: Story = { args: { route: "appearance", locale: "zh-CN" } };
export const Llm: Story = { args: { route: "llm", locale: "zh-CN" } };
export const Avatar: Story = { args: { route: "avatar", locale: "zh-CN" } };
export const Voice: Story = { args: { route: "voice", locale: "zh-CN" } };
