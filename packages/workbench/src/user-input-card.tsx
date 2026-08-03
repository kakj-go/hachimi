import type {
  UserInputAnswer,
  UserInputRequestRecord,
  UserInputResolutionAction,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, MessageCircle, TextField } from "@hachimi/ui";
import { For, Show, createMemo, createSignal, onCleanup, untrack } from "solid-js";

export function UserInputCard(props: {
  request: UserInputRequestRecord;
  resolving: boolean;
  onResolve: (
    request: UserInputRequestRecord,
    answers: UserInputAnswer[],
    action: UserInputResolutionAction,
  ) => void;
}) {
  const i18n = useI18n();
  const [answers, setAnswers] = createSignal<Record<string, string>>(
    untrack(() =>
      Object.fromEntries(
        props.request.questions.map((question) => [
          question.id,
          question.defaultAnswer ?? question.options[0]?.value ?? "",
        ]),
      ),
    ),
  );
  const [freeForm, setFreeForm] = createSignal<Record<string, boolean>>({});
  const [questionIndex, setQuestionIndex] = createSignal(0);
  const question = createMemo(() => props.request.questions[questionIndex()]);
  const [now, setNow] = createSignal(Date.now());
  const timer = window.setInterval(() => setNow(Date.now()), 1_000);
  onCleanup(() => window.clearInterval(timer));
  const remainingSeconds = createMemo(() => {
    if (!props.request.expiresAtMs) return undefined;
    return Math.max(0, Math.ceil((props.request.expiresAtMs - now()) / 1_000));
  });
  const complete = () =>
    props.request.questions.every((question) => (answers()[question.id] ?? "").trim().length > 0);

  return (
    <article class="user-input-card agent-card approval" data-component="user-input-card">
      <header>
        <MessageCircle size={17} />
        <span>
          <strong>{i18n.locale() === "zh-CN" ? "需要你的输入" : "Your input is needed"}</strong>
          <small>
            {i18n.locale() === "zh-CN"
              ? "回答只会交给当前运行；密钥类回答不会写入历史"
              : "Answers go only to the active run; secret answers are never persisted"}
          </small>
          <Show when={remainingSeconds() !== undefined}>
            <small class="user-input-countdown">
              {i18n.locale() === "zh-CN" ? "自动选择倒计时" : "Auto-select in"} {remainingSeconds()}
              s
            </small>
          </Show>
        </span>
        <Show when={props.request.questions.length > 1}>
          <nav
            class="user-input-pager"
            aria-label={i18n.locale() === "zh-CN" ? "问题导航" : "Question navigation"}
          >
            <Button
              size="small"
              variant="ghost"
              aria-label={i18n.locale() === "zh-CN" ? "上一个问题" : "Previous question"}
              disabled={questionIndex() === 0}
              onClick={() => setQuestionIndex((current) => Math.max(0, current - 1))}
            >
              ‹
            </Button>
            <small>
              {questionIndex() + 1} of {props.request.questions.length}
            </small>
            <Button
              size="small"
              variant="ghost"
              aria-label={i18n.locale() === "zh-CN" ? "下一个问题" : "Next question"}
              disabled={questionIndex() >= props.request.questions.length - 1}
              onClick={() =>
                setQuestionIndex((current) =>
                  Math.min(props.request.questions.length - 1, current + 1),
                )
              }
            >
              ›
            </Button>
          </nav>
        </Show>
      </header>
      <Show when={question()} keyed>
        {(question) => (
          <div class="user-input-question">
            <div class="user-input-question-heading">
              <strong>{question.header}</strong>
              <span>{question.prompt}</span>
            </div>
            <Show when={question.options.length > 0}>
              <div class="user-input-choices" role="radiogroup" aria-label={question.header}>
                <For each={question.options}>
                  {(option, index) => (
                    <Button
                      type="button"
                      classList={{
                        selected:
                          !freeForm()[question.id] && answers()[question.id] === option.value,
                      }}
                      role="radio"
                      aria-checked={
                        !freeForm()[question.id] && answers()[question.id] === option.value
                      }
                      onClick={() => {
                        setFreeForm((current) => ({ ...current, [question.id]: false }));
                        setAnswers((current) => ({ ...current, [question.id]: option.value }));
                        if (questionIndex() < props.request.questions.length - 1) {
                          setQuestionIndex((current) => current + 1);
                        }
                      }}
                    >
                      <span class="choice-index">{index() + 1}</span>
                      <span>
                        <strong>{option.label}</strong>
                        <Show when={option.description}>{(value) => <small>{value()}</small>}</Show>
                      </span>
                    </Button>
                  )}
                </For>
                <Button
                  type="button"
                  classList={{ selected: freeForm()[question.id] }}
                  role="radio"
                  aria-checked={Boolean(freeForm()[question.id])}
                  onClick={() => {
                    setFreeForm((current) => ({ ...current, [question.id]: true }));
                    setAnswers((current) => ({ ...current, [question.id]: "" }));
                  }}
                >
                  <span class="choice-index">+</span>
                  <span>
                    <strong>{i18n.locale() === "zh-CN" ? "自由输入" : "Free-form answer"}</strong>
                  </span>
                </Button>
              </div>
            </Show>
            <Show when={question.options.length === 0 || freeForm()[question.id]}>
              <TextField
                label={i18n.locale() === "zh-CN" ? "你的回答" : "Your answer"}
                type={question.secret ? "password" : "text"}
                value={answers()[question.id] ?? ""}
                placeholder={i18n.locale() === "zh-CN" ? "输入回答" : "Enter an answer"}
                onInput={(event) =>
                  setAnswers((current) => ({
                    ...current,
                    [question.id]: event.currentTarget.value,
                  }))
                }
              />
            </Show>
          </div>
        )}
      </Show>
      <footer>
        <Button
          size="small"
          variant="ghost"
          data-testid="workbench-decline-user-input"
          disabled={props.resolving}
          onClick={() => props.onResolve(props.request, [], "decline")}
        >
          {i18n.locale() === "zh-CN" ? "拒绝提供" : "Decline"}
        </Button>
        <Button
          size="small"
          variant="ghost"
          data-testid="workbench-cancel-user-input"
          disabled={props.resolving}
          onClick={() => props.onResolve(props.request, [], "cancel")}
        >
          {i18n.locale() === "zh-CN" ? "取消请求" : "Cancel request"}
        </Button>
        <Button
          size="small"
          variant="primary"
          data-testid="workbench-submit-user-input"
          disabled={props.resolving || !complete()}
          onClick={() =>
            props.onResolve(
              props.request,
              props.request.questions.map((question) => ({
                questionId: question.id,
                value: answers()[question.id] ?? "",
              })),
              "submit",
            )
          }
        >
          {i18n.locale() === "zh-CN" ? "提交回答" : "Submit answers"}
        </Button>
      </footer>
    </article>
  );
}
