import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Show } from "solid-js";
import { FloatingIconButton, MessageCircle, Mic2, Send, Volume2, VolumeX } from "../index";

type PetState = "fallback" | "loading" | "input" | "reply" | "muted" | "error";

function PetPreview(props: { state: PetState }) {
  const muted = () => props.state === "muted";
  return (
    <div
      style={{
        width: "360px",
        height: "480px",
        position: "relative",
        overflow: "hidden",
        background:
          "linear-gradient(45deg, #ddd 25%, transparent 25%) 0 0 / 20px 20px, linear-gradient(-45deg, #ddd 25%, transparent 25%) 0 10px / 20px 20px, linear-gradient(45deg, transparent 75%, #ddd 75%) 10px -10px / 20px 20px, linear-gradient(-45deg, transparent 75%, #ddd 75%) -10px 0 / 20px 20px, #eee",
      }}
    >
      <Show when={["reply", "error"].includes(props.state)}>
        <div
          role="status"
          style={{
            position: "absolute",
            top: "18px",
            left: "28px",
            right: "28px",
            padding: "12px 14px",
            border: "1px solid var(--color-border-base)",
            "border-radius": "16px",
            background: "var(--color-bg-floating)",
            color: props.state === "error" ? "var(--color-state-danger)" : "var(--color-text-base)",
            "box-shadow": "var(--shadow-floating)",
          }}
        >
          {props.state === "error"
            ? "无法连接到大语言模型服务。"
            : "当然可以。今天也一起愉快地工作吧！"}
        </div>
      </Show>
      <div
        style={{
          position: "absolute",
          left: "58px",
          right: "58px",
          bottom: props.state === "input" ? "116px" : "64px",
          height: props.state === "input" ? "286px" : "338px",
          display: "grid",
          "place-items": "center",
          filter: "drop-shadow(0 16px 20px rgb(25 20 50 / 20%))",
        }}
      >
        <div
          style={{
            width: "178px",
            height: "260px",
            "border-radius": "46% 46% 38% 38% / 32% 32% 52% 52%",
            background: "linear-gradient(145deg, #d6d1ff, #7564de)",
            opacity: props.state === "loading" ? 0.45 : 1,
          }}
        />
        <Show when={props.state === "loading"}>
          <span style={{ position: "absolute", color: "var(--color-text-base)" }}>
            Loading GLB…
          </span>
        </Show>
      </div>
      <Show when={props.state === "input"}>
        <div
          style={{
            position: "absolute",
            left: "18px",
            right: "18px",
            bottom: "60px",
            display: "grid",
            "grid-template-columns": "1fr 34px 36px",
            gap: "8px",
            padding: "10px 10px 10px 14px",
            border: "1px solid var(--color-border-base)",
            "border-radius": "17px",
            background: "var(--color-bg-floating)",
            "box-shadow": "var(--shadow-floating)",
          }}
        >
          <span style={{ color: "var(--color-text-faint)", "align-self": "center" }}>
            和 Hachimi 说点什么…
          </span>
          <button
            type="button"
            aria-label="语音输入"
            style={{ border: "0", background: "transparent", color: "var(--color-text-muted)" }}
          >
            <Mic2 size={16} />
          </button>
          <FloatingIconButton label="发送">
            <Send size={16} />
          </FloatingIconButton>
        </div>
      </Show>
      <Show when={props.state !== "input"}>
        <div
          style={{
            position: "absolute",
            left: "0",
            right: "0",
            bottom: "18px",
            display: "flex",
            "justify-content": "center",
            gap: "9px",
          }}
        >
          <FloatingIconButton
            label="消息"
            style={{ background: "transparent", border: "0", "box-shadow": "none" }}
          >
            <MessageCircle size={17} />
          </FloatingIconButton>
          <FloatingIconButton
            label={muted() ? "取消静音" : "静音"}
            aria-pressed={muted()}
            style={{ background: "transparent", border: "0", "box-shadow": "none" }}
          >
            <Show when={muted()} fallback={<Volume2 size={17} />}>
              <VolumeX size={17} />
            </Show>
          </FloatingIconButton>
        </div>
      </Show>
    </div>
  );
}

const meta = {
  title: "Examples/Pet",
  component: PetPreview,
} satisfies Meta<typeof PetPreview>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Fallback: Story = { args: { state: "fallback" } };
export const GlbLoading: Story = { args: { state: "loading" } };
export const Input: Story = { args: { state: "input" } };
export const Reply: Story = { args: { state: "reply" } };
export const Muted: Story = { args: { state: "muted" } };
export const Error: Story = { args: { state: "error" } };
