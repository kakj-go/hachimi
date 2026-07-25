import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Button, IconButton } from "../index";

const meta = {
  title: "Components/Button",
  component: Button,
  args: {
    children: "Button",
    variant: "default",
    size: "normal",
  },
  argTypes: {
    variant: { control: "select", options: ["default", "primary", "ghost", "danger"] },
    size: { control: "select", options: ["small", "normal", "large"] },
  },
} satisfies Meta<typeof Button>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Primary: Story = { args: { variant: "primary", children: "Confirm" } };
export const Disabled: Story = { args: { disabled: true } };
export const Icon: Story = {
  render: () => <IconButton label="Close">×</IconButton>,
};
