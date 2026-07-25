import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";
import { Button, Dialog } from "../index";

function DialogExample() {
  const [open, setOpen] = createSignal(false);
  return (
    <>
      <Button variant="primary" onClick={() => setOpen(true)}>
        Open dialog
      </Button>
      <Dialog
        open={open()}
        onOpenChange={setOpen}
        title="Confirm action"
        description="The dialog traps focus and closes with Escape."
      >
        <Button onClick={() => setOpen(false)}>Confirm</Button>
      </Dialog>
    </>
  );
}

const meta = {
  title: "Components/Dialog",
  component: DialogExample,
} satisfies Meta<typeof DialogExample>;

export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
