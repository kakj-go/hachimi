import type {
  ApprovalRequestRecord,
  ApprovalStatus,
  HostAccessDecision,
  HostAccessRequestRecord,
  ProposedPlan,
  UserInputAnswer,
  UserInputRequestRecord,
  UserInputResolutionAction,
} from "@hachimi/contracts";
import {
  ApprovalCard,
  ArrowRight,
  Button,
  CircleHelp,
  Send,
  ShieldAlert,
  ShieldCheck,
  TextArea,
} from "@hachimi/ui";
import { For, Show, createSignal } from "solid-js";

import { UserInputCard } from "../user-input-card";

export type WorkbenchGateProps = {
  userInput: UserInputRequestRecord | undefined;
  hostAccess: HostAccessRequestRecord | undefined;
  approval: ApprovalRequestRecord | undefined;
  plan: ProposedPlan | undefined;
  resolvingUserInput: boolean;
  resolvingHostAccess: boolean;
  resolvingApproval: boolean;
  acceptingPlan: boolean;
  revisingPlan: boolean;
  locale: "zh-CN" | "en-US";
  onResolveUserInput: (
    request: UserInputRequestRecord,
    answers: UserInputAnswer[],
    action: UserInputResolutionAction,
  ) => void;
  onResolveApproval: (approval: ApprovalRequestRecord, decision: ApprovalStatus) => void;
  onResolveHostAccess: (request: HostAccessRequestRecord, decision: HostAccessDecision) => void;
  onAcceptPlan: (plan: ProposedPlan) => void;
  onRevisePlan: (plan: ProposedPlan, instructions: string) => void;
  onDismissPlan: (plan: ProposedPlan) => void;
};

export function WorkbenchGate(props: WorkbenchGateProps) {
  const [editingPlan, setEditingPlan] = createSignal(false);
  const [instructions, setInstructions] = createSignal("");
  const zh = () => props.locale === "zh-CN";

  return (
    <section class="workbench-gate" aria-live="polite">
      <Show when={props.userInput}>
        {(request) => (
          <UserInputCard
            request={request()}
            resolving={props.resolvingUserInput}
            onResolve={props.onResolveUserInput}
          />
        )}
      </Show>
      <Show when={!props.userInput && props.hostAccess}>
        {(request) => (
          <ApprovalCard
            title={hostAccessTitle(request(), props.locale)}
            description={
              zh()
                ? "此授权只允许 Agent 访问目标，不会跳过发送、删除、购买或下载等操作审批。"
                : "This grant only allows target access. Send, delete, purchase, and download actions keep their own approvals."
            }
            icon={<ShieldAlert size={17} />}
            actions={
              <>
                <Button
                  data-testid="workbench-host-deny"
                  disabled={props.resolvingHostAccess}
                  onClick={() => props.onResolveHostAccess(request(), "deny")}
                >
                  {zh() ? "拒绝" : "Deny"}
                </Button>
                <Button
                  data-testid="workbench-host-allow-once"
                  disabled={props.resolvingHostAccess}
                  onClick={() => props.onResolveHostAccess(request(), "allow_once")}
                >
                  {zh() ? "允许一次" : "Allow once"}
                </Button>
                <Button
                  data-testid="workbench-host-allow-session"
                  variant="primary"
                  disabled={props.resolvingHostAccess}
                  onClick={() => props.onResolveHostAccess(request(), "allow_session")}
                >
                  {zh() ? "本会话允许" : "Allow for session"}
                </Button>
                <Button
                  data-testid="workbench-host-always-allow"
                  disabled={props.resolvingHostAccess || isPrivateBrowserRequest(request())}
                  onClick={() => props.onResolveHostAccess(request(), "always_allow")}
                >
                  {zh() ? "始终允许" : "Always allow"}
                </Button>
                <Button
                  data-testid="workbench-host-always-block"
                  disabled={props.resolvingHostAccess}
                  onClick={() => props.onResolveHostAccess(request(), "always_block")}
                >
                  {zh() ? "始终阻止" : "Always block"}
                </Button>
              </>
            }
          >
            <dl class="approval-request-details" data-testid="host-access-request-details">
              <div>
                <dt>{zh() ? "目标" : "Target"}</dt>
                <dd>
                  <code>{hostAccessTarget(request())}</code>
                </dd>
              </div>
              <div>
                <dt>{zh() ? "访问能力" : "Access"}</dt>
                <dd class="approval-scope-list">
                  <For each={request().capabilities}>
                    {(capability) => <code>{capability}</code>}
                  </For>
                </dd>
              </div>
              <div>
                <dt>{zh() ? "运行" : "Run"}</dt>
                <dd>
                  <code>{request().ownerRunId}</code>
                </dd>
              </div>
            </dl>
          </ApprovalCard>
        )}
      </Show>
      <Show when={!props.userInput && !props.hostAccess && props.approval}>
        {(approval) => (
          <ApprovalCard
            title={
              zh()
                ? `允许 Agent 执行「${humanizeApprovalAction(approval().action)}」？`
                : `Allow the agent to ${humanizeApprovalAction(approval().action)}?`
            }
            description={approval().riskSummary}
            icon={<ShieldCheck size={17} />}
            actions={
              <>
                <Button
                  data-testid="workbench-deny-approval"
                  disabled={props.resolvingApproval}
                  onClick={() => props.onResolveApproval(approval(), "denied")}
                >
                  {zh() ? "拒绝" : "Deny"}
                </Button>
                <Button
                  data-testid="workbench-approve-once"
                  variant="primary"
                  disabled={props.resolvingApproval}
                  onClick={() => props.onResolveApproval(approval(), "approved")}
                >
                  {approvalDecisionLabel(approval(), props.locale)}
                </Button>
              </>
            }
          >
            <dl class="approval-request-details" data-testid="approval-request-details">
              <div>
                <dt>{zh() ? "操作" : "Action"}</dt>
                <dd>
                  <code>{approval().action}</code>
                </dd>
              </div>
              <div>
                <dt>{zh() ? "目标" : "Target"}</dt>
                <dd>
                  <code>{approval().resource}</code>
                </dd>
              </div>
              <div>
                <dt>{zh() ? "执行位置" : "Host"}</dt>
                <dd>{approval().targetHost}</dd>
              </div>
              <div>
                <dt>{zh() ? "所需权限" : "Permissions"}</dt>
                <dd class="approval-scope-list">
                  <Show
                    when={approval().requiredScopes.length > 0}
                    fallback={
                      <span>{zh() ? "未声明额外权限" : "No additional scopes declared"}</span>
                    }
                  >
                    <For each={approval().requiredScopes}>{(scope) => <code>{scope}</code>}</For>
                  </Show>
                </dd>
              </div>
              <div>
                <dt>{zh() ? "授权范围" : "Grant"}</dt>
                <dd>{approvalGrantDescription(approval(), props.locale)}</dd>
              </div>
              <div>
                <dt>{zh() ? "请求方" : "Requested by"}</dt>
                <dd>{approval().requesterPrincipal}</dd>
              </div>
              <Show when={approval().expiresAtMs}>
                {(expiresAtMs) => (
                  <div>
                    <dt>{zh() ? "到期时间" : "Expires"}</dt>
                    <dd>{formatApprovalExpiry(expiresAtMs(), props.locale)}</dd>
                  </div>
                )}
              </Show>
            </dl>
          </ApprovalCard>
        )}
      </Show>
      <Show when={!props.userInput && !props.hostAccess && !props.approval && props.plan}>
        {(plan) => (
          <article class="plan-confirmation-gate">
            <header>
              <span>
                <CircleHelp size={17} />
                <strong>{zh() ? "实施此计划？" : "Implement this plan?"}</strong>
              </span>
              <small>{zh() ? `修订版 ${plan().revision}` : `Revision ${plan().revision}`}</small>
            </header>
            <Show
              when={editingPlan()}
              fallback={
                <div class="plan-confirmation-options">
                  <Button
                    data-testid="workbench-execute-plan"
                    disabled={props.acceptingPlan}
                    onClick={() => props.onAcceptPlan(plan())}
                  >
                    <span class="choice-index">1</span>
                    <span>
                      <strong>{zh() ? "是，实施此计划" : "Yes, implement this plan"}</strong>
                    </span>
                    <ArrowRight size={16} />
                  </Button>
                  <Button data-testid="workbench-revise-plan" onClick={() => setEditingPlan(true)}>
                    <span class="choice-index">2</span>
                    <span>
                      <strong>
                        {zh()
                          ? "否，并告诉 Agent 应该如何更改"
                          : "No, tell the agent what to change"}
                      </strong>
                    </span>
                    <ArrowRight size={16} />
                  </Button>
                  <Button
                    class="plan-confirmation-skip"
                    data-testid="workbench-dismiss-plan"
                    variant="ghost"
                    onClick={() => props.onDismissPlan(plan())}
                  >
                    {zh() ? "跳过" : "Skip"}
                  </Button>
                </div>
              }
            >
              <TextArea
                class="plan-revision-field"
                label={zh() ? "告诉 Agent 如何更改计划" : "Tell the agent what to change"}
                autofocus
                value={instructions()}
                onInput={(event) => setInstructions(event.currentTarget.value)}
              />
              <footer>
                <Button
                  data-testid="workbench-cancel-plan-revision"
                  disabled={props.revisingPlan}
                  onClick={() => {
                    setEditingPlan(false);
                    setInstructions("");
                  }}
                >
                  {zh() ? "取消" : "Cancel"}
                </Button>
                <Button
                  data-testid="workbench-submit-plan-revision"
                  variant="primary"
                  disabled={props.revisingPlan || !instructions().trim()}
                  onClick={() => props.onRevisePlan(plan(), instructions().trim())}
                >
                  <Send size={14} /> {zh() ? "提交更改" : "Submit revision"}
                </Button>
              </footer>
            </Show>
          </article>
        )}
      </Show>
    </section>
  );
}

function hostAccessTarget(request: HostAccessRequestRecord) {
  return request.target.kind === "browser"
    ? request.target.origin
    : request.target.app.displayName || request.target.app.appId;
}

function isPrivateBrowserRequest(request: HostAccessRequestRecord) {
  return request.target.kind === "browser" && request.target.private_network;
}

function hostAccessTitle(request: HostAccessRequestRecord, locale: WorkbenchGateProps["locale"]) {
  const target = hostAccessTarget(request);
  if (request.target.kind === "browser") {
    return locale === "zh-CN"
      ? `允许 Agent 访问网站「${target}」？`
      : `Allow the agent to access ${target}?`;
  }
  return locale === "zh-CN"
    ? `允许 Agent 控制应用「${target}」？`
    : `Allow the agent to control ${target}?`;
}

function humanizeApprovalAction(action: string) {
  return action.replace(/[_-]+/g, " ").trim();
}

function approvalDecisionLabel(
  approval: ApprovalRequestRecord,
  locale: WorkbenchGateProps["locale"],
) {
  const zh = locale === "zh-CN";
  if (approval.grantScope === "session") return zh ? "允许本次会话" : "Allow for session";
  if (approval.grantScope === "timed_lease") return zh ? "临时允许" : "Allow temporarily";
  return zh ? "允许一次" : "Allow once";
}

function approvalGrantDescription(
  approval: ApprovalRequestRecord,
  locale: WorkbenchGateProps["locale"],
) {
  const zh = locale === "zh-CN";
  if (approval.grantScope === "session") {
    return zh ? "在当前会话内有效" : "Valid for the current session";
  }
  if (approval.grantScope === "timed_lease") {
    return zh ? "在到期前临时有效" : "Temporarily valid until expiry";
  }
  return zh
    ? `仅本次请求有效（可用 ${approval.usesRemaining} 次）`
    : `This request only (${approval.usesRemaining} use${approval.usesRemaining === 1 ? "" : "s"})`;
}

function formatApprovalExpiry(expiresAtMs: number, locale: WorkbenchGateProps["locale"]) {
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(expiresAtMs));
}
