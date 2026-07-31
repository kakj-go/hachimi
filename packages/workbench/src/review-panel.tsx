import {
  commandFailure,
  type ReviewDelivery,
  type ReviewFindingStatus,
  type ReviewSeverity,
  type ReviewSnapshot,
  type ReviewTarget,
  type SessionRecord,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  AlertTriangle,
  Badge,
  Bug,
  Button,
  Check,
  GitPullRequest,
  RefreshCw,
  SelectField,
  ShieldCheck,
  TextField,
  X,
} from "@hachimi/ui";
import { For, Show, createEffect, createMemo, createSignal, untrack } from "solid-js";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { directUserMutationContext, runMutationContext } from "./mutation-context";
import "./review-panel.css";

type ReviewTargetKind = ReviewTarget["kind"];

const TERMINAL_RUN_STATUSES = new Set(["succeeded", "failed", "cancelled", "interrupted", "lost"]);

function severityTone(severity: ReviewSeverity): "info" | "warning" | "danger" {
  if (severity === "critical" || severity === "error") return "danger";
  return severity === "warning" ? "warning" : "info";
}

function reviewTargetLabel(target: ReviewTarget, zh: boolean): string {
  switch (target.kind) {
    case "uncommitted_changes":
      return zh ? "未提交变更" : "Uncommitted changes";
    case "base_branch":
      return `${zh ? "基础分支" : "Base branch"}: ${target.value}`;
    case "commit":
      return `${zh ? "提交" : "Commit"}: ${target.value}`;
    case "custom":
      return zh ? "自定义检查" : "Custom review";
  }
}

export function ReviewPanel(props: {
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  onOpenSession: (session: SessionRecord) => void;
}) {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [open, setOpen] = createSignal(false);
  const [targetKind, setTargetKind] = createSignal<ReviewTargetKind>("uncommitted_changes");
  const [targetValue, setTargetValue] = createSignal("");
  const [delivery, setDelivery] = createSignal<ReviewDelivery>("inline");
  const [reviews, setReviews] = createSignal<ReviewSnapshot[]>([]);
  const [selectedReviewId, setSelectedReviewId] = createSignal<string>();
  const [loading, setLoading] = createSignal(false);
  const [starting, setStarting] = createSignal(false);
  const [busyFindingId, setBusyFindingId] = createSignal<string>();
  const [failure, setFailure] = createSignal<string>();
  let loadGeneration = 0;

  const sourceRun = createMemo(() => props.snapshot.runs[props.snapshot.runs.length - 1]);
  const sourceReady = createMemo(() => {
    const run = sourceRun();
    return Boolean(run && TERMINAL_RUN_STATUSES.has(run.status));
  });
  const selectedReview = createMemo(
    () => reviews().find((review) => review.review.id === selectedReviewId()) ?? reviews()[0],
  );

  async function refresh(sessionId = props.snapshot.session.id, preferredId?: string) {
    const generation = ++loadGeneration;
    setLoading(true);
    try {
      const next = await props.commandPort.listReviews(sessionId);
      if (generation !== loadGeneration || untrack(() => props.snapshot.session.id) !== sessionId)
        return;
      setReviews(next);
      setSelectedReviewId((current) =>
        preferredId && next.some((review) => review.review.id === preferredId)
          ? preferredId
          : current && next.some((review) => review.review.id === current)
            ? current
            : next[0]?.review.id,
      );
      setFailure(undefined);
    } catch (error) {
      if (generation === loadGeneration) setFailure(commandFailure(error).message);
    } finally {
      if (generation === loadGeneration) setLoading(false);
    }
  }

  createEffect(() => {
    const sessionId = props.snapshot.session.id;
    const runRevision = props.snapshot.runs
      .map((run) => `${run.id}:${run.status}:${run.updatedAtMs}`)
      .join("|");
    void runRevision;
    void refresh(sessionId);
  });

  function buildTarget(): ReviewTarget | undefined {
    const kind = targetKind();
    if (kind === "uncommitted_changes") return { kind };
    const value = targetValue().trim();
    return value ? { kind, value } : undefined;
  }

  async function start() {
    const run = sourceRun();
    const target = buildTarget();
    if (!run || !sourceReady()) {
      setFailure(
        zh()
          ? "请等待当前 Run 结束后再启动只读 Review。"
          : "Wait for the current Run to finish before starting a read-only Review.",
      );
      return;
    }
    if (!target) {
      setFailure(zh() ? "请输入 Review 目标。" : "Enter the Review target.");
      return;
    }
    setStarting(true);
    setFailure(undefined);
    try {
      const started = await props.commandPort.startReview({
        context: runMutationContext(run),
        sessionId: props.snapshot.session.id,
        target,
        delivery: delivery(),
      });
      setOpen(true);
      if (started.review.delivery === "detached") {
        props.onOpenSession(started.session);
      } else {
        await refresh(started.session.id, started.review.id);
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setStarting(false);
    }
  }

  async function updateFinding(status: ReviewFindingStatus, findingId: string) {
    const selected = selectedReview();
    if (!selected) return;
    setBusyFindingId(findingId);
    setFailure(undefined);
    try {
      const finding = await props.commandPort.updateReviewFinding({
        context: directUserMutationContext(),
        reviewId: selected.review.id,
        findingId,
        status,
      });
      setReviews((current) =>
        current.map((review) =>
          review.review.id === selected.review.id
            ? {
                ...review,
                findings: review.findings.map((candidate) =>
                  candidate.id === finding.id ? finding : candidate,
                ),
              }
            : review,
        ),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusyFindingId(undefined);
    }
  }

  return (
    <section class="review-panel" data-testid="review-panel">
      <header class="review-panel-heading">
        <div>
          <GitPullRequest size={16} />
          <strong>{zh() ? "代码 Review" : "Code Review"}</strong>
          <Badge tone="info">{zh() ? "只读" : "Read-only"}</Badge>
        </div>
        <div class="review-heading-actions">
          <Button
            size="small"
            aria-label={zh() ? "刷新 Review" : "Refresh Reviews"}
            disabled={loading()}
            onClick={() => void refresh()}
          >
            <RefreshCw size={14} />
          </Button>
          <Button
            size="small"
            data-testid="review-toggle"
            onClick={() => setOpen((value) => !value)}
          >
            {open() ? (zh() ? "收起" : "Collapse") : zh() ? "打开" : "Open"}
          </Button>
        </div>
      </header>

      <Show when={open()}>
        <div class="review-start-form">
          <SelectField
            label={zh() ? "检查目标" : "Review target"}
            testId="review-target-kind"
            value={targetKind()}
            options={[
              { value: "uncommitted_changes", label: zh() ? "未提交变更" : "Uncommitted changes" },
              { value: "base_branch", label: zh() ? "与基础分支比较" : "Compare with base branch" },
              { value: "commit", label: zh() ? "单个提交" : "Single commit" },
              { value: "custom", label: zh() ? "自定义检查" : "Custom review" },
            ]}
            onChange={(value) => setTargetKind(value as ReviewTargetKind)}
          />
          <Show when={targetKind() !== "uncommitted_changes"}>
            <TextField
              label={
                targetKind() === "custom"
                  ? zh()
                    ? "检查要求"
                    : "Review instructions"
                  : zh()
                    ? "Git 引用"
                    : "Git revision"
              }
              testId="review-target-value"
              value={targetValue()}
              placeholder={
                targetKind() === "base_branch" ? "main" : targetKind() === "commit" ? "HEAD~1" : ""
              }
              onInput={(event) => setTargetValue(event.currentTarget.value)}
            />
          </Show>
          <SelectField
            label={zh() ? "交付方式" : "Delivery"}
            testId="review-delivery"
            value={delivery()}
            options={[
              { value: "inline", label: zh() ? "当前 Session" : "Current Session" },
              { value: "detached", label: zh() ? "独立 Session" : "Detached Session" },
            ]}
            onChange={(value) => setDelivery(value as ReviewDelivery)}
          />
          <Button
            variant="primary"
            data-testid="review-start"
            disabled={!sourceReady() || starting() || !buildTarget()}
            loading={starting()}
            onClick={() => void start()}
          >
            <Bug size={15} /> {zh() ? "开始 Review" : "Start Review"}
          </Button>
        </div>

        <Show when={failure()}>
          {(message) => (
            <p class="review-failure" role="alert">
              <AlertTriangle size={14} /> {message()}
            </p>
          )}
        </Show>

        <Show
          when={reviews().length > 0}
          fallback={
            <p class="review-empty">
              {loading()
                ? zh()
                  ? "正在读取 Review…"
                  : "Loading Reviews…"
                : zh()
                  ? "还没有 Review。"
                  : "No Reviews yet."}
            </p>
          }
        >
          <div
            class="review-history"
            role="tablist"
            aria-label={zh() ? "Review 历史" : "Review history"}
          >
            <For each={reviews()}>
              {(review) => (
                <Button
                  size="small"
                  classList={{ active: selectedReview()?.review.id === review.review.id }}
                  data-testid={`review-history-${review.review.id}`}
                  onClick={() => setSelectedReviewId(review.review.id)}
                >
                  {reviewTargetLabel(review.review.target, zh())}
                  <Badge
                    tone={
                      review.run.status === "succeeded"
                        ? "success"
                        : review.run.status === "failed"
                          ? "danger"
                          : "neutral"
                    }
                  >
                    {review.run.status}
                  </Badge>
                </Button>
              )}
            </For>
          </div>

          <Show when={selectedReview()}>
            {(review) => (
              <div class="review-result" data-testid="review-result">
                <header>
                  <div>
                    <ShieldCheck size={16} />
                    <strong>{reviewTargetLabel(review().review.target, zh())}</strong>
                  </div>
                  <span>
                    {review().overallCorrectness ?? review().run.status}
                    <Show when={review().overallConfidenceScore != null}>
                      {` · ${Math.round((review().overallConfidenceScore ?? 0) * 100)}%`}
                    </Show>
                  </span>
                </header>
                <Show when={review().summary}>
                  {(summary) => <p class="review-summary">{summary()}</p>}
                </Show>
                <Show
                  when={review().findings.length > 0}
                  fallback={
                    <p class="review-empty">
                      {review().run.status === "succeeded"
                        ? zh()
                          ? "没有发现可操作的问题。"
                          : "No actionable findings."
                        : zh()
                          ? "Review 正在运行或尚未生成结果。"
                          : "Review is running or has not produced a result yet."}
                    </p>
                  }
                >
                  <div class="review-findings">
                    <For each={review().findings}>
                      {(finding) => (
                        <article
                          class="review-finding"
                          data-severity={finding.severity}
                          data-testid="review-finding"
                        >
                          <header>
                            <Badge tone={severityTone(finding.severity)}>{finding.severity}</Badge>
                            <strong>{finding.message}</strong>
                            <span>{finding.status}</span>
                          </header>
                          <Show when={finding.file}>
                            {(file) => (
                              <code>{`${file()}${finding.line ? `:${finding.line}` : ""}`}</code>
                            )}
                          </Show>
                          <p>{finding.evidence}</p>
                          <footer>
                            <Button
                              size="small"
                              data-testid={`review-finding-acknowledge-${finding.id}`}
                              disabled={
                                busyFindingId() === finding.id || finding.status === "acknowledged"
                              }
                              onClick={() => void updateFinding("acknowledged", finding.id)}
                            >
                              <Check size={13} /> {zh() ? "已查看" : "Acknowledge"}
                            </Button>
                            <Button
                              size="small"
                              data-testid={`review-finding-resolve-${finding.id}`}
                              disabled={
                                busyFindingId() === finding.id || finding.status === "resolved"
                              }
                              onClick={() => void updateFinding("resolved", finding.id)}
                            >
                              <ShieldCheck size={13} /> {zh() ? "已解决" : "Resolve"}
                            </Button>
                            <Button
                              size="small"
                              disabled={
                                busyFindingId() === finding.id || finding.status === "dismissed"
                              }
                              onClick={() => void updateFinding("dismissed", finding.id)}
                            >
                              <X size={13} /> {zh() ? "忽略" : "Dismiss"}
                            </Button>
                          </footer>
                        </article>
                      )}
                    </For>
                  </div>
                </Show>
              </div>
            )}
          </Show>
        </Show>
      </Show>
    </section>
  );
}
