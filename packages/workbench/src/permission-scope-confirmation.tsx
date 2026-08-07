import { Button, Dialog, StatusBanner } from "@hachimi/ui";
import type { AgentPermissionPolicy } from "@hachimi/contracts";

import { permissionScopeRisk } from "./permission-scope-risk";

export function PermissionScopeConfirmation(props: {
  open: boolean;
  policy: AgentPermissionPolicy;
  zh: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const risk = () => permissionScopeRisk(props.policy);
  return (
    <Dialog
      open={props.open}
      tone="danger"
      title={props.zh ? "确认不限制的权限范围" : "Confirm unrestricted permission scopes"}
      description={
        props.zh
          ? "保存后，Agent 可在所选资源类型中访问任意目标。"
          : "After saving, the Agent can access any target in the selected resource categories."
      }
      closeLabel={props.zh ? "关闭" : "Close"}
      onOpenChange={(open) => !open && props.onCancel()}
    >
      <div class="permission-scope-confirmation">
        <StatusBanner tone="danger">
          {risk().equivalentToFullAccess
            ? props.zh
              ? "当前范围和能力组合等价于完全授权。"
              : "The current scope and capability combination is equivalent to Full access."
            : props.zh
              ? "至少一个资源范围已设为全部。请确认这是预期授权。"
              : "At least one resource scope allows all targets. Confirm that this is intentional."}
        </StatusBanner>
        <p>
          {props.zh
            ? "文件拒绝规则、操作系统沙箱、浏览器站点确认、提权窗口和 Hachimi 自身窗口拦截不会被绕过。"
            : "File deny rules, OS sandboxing, browser site confirmation, elevated-window blocks, and Hachimi self-window blocks remain enforced."}
        </p>
        <div class="dialog-actions">
          <Button variant="ghost" onClick={props.onCancel}>
            {props.zh ? "返回检查" : "Review settings"}
          </Button>
          <Button variant="danger" onClick={props.onConfirm}>
            {props.zh ? "确认并保存" : "Confirm and save"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
