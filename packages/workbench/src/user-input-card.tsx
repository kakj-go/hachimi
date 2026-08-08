import type {
  UserInputAnswer,
  UserInputRequestRecord,
  UserInputResolutionAction,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, ChevronLeft, ChevronRight, Pencil, TextField, X } from "@hachimi/ui";
import { For, Show, createMemo, createSignal, untrack } from "solid-js";

import "./user-input-card.css";

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
  const zh = () => i18n.locale() === "zh-CN";
  const requestQuestions = untrack(() => props.request.questions);
  const initialAnswers = untrack(() =>
    Object.fromEntries(requestQuestions.map((item) => [item.id, item.defaultAnswer ?? ""])),
  );
  const initialOther = untrack(() =>
    Object.fromEntries(
      requestQuestions.map((item) => [
        item.id,
        Boolean(
          item.defaultAnswer && !item.options.some((option) => option.value === item.defaultAnswer),
        ),
      ]),
    ),
  );
  const [answers, setAnswers] = createSignal<Record<string, string>>(initialAnswers);
  const [otherAnswers, setOtherAnswers] = createSignal<Record<string, string>>(
    Object.fromEntries(
      requestQuestions.map((item) => [
        item.id,
        initialOther[item.id] ? (item.defaultAnswer ?? "") : "",
      ]),
    ),
  );
  const [other, setOther] = createSignal<Record<string, boolean>>(initialOther);
  const [questionIndex, setQuestionIndex] = createSignal(0);
  const question = createMemo(() => requestQuestions[questionIndex()]);
  const complete = () =>
    requestQuestions.every((item) => (answers()[item.id] ?? "").trim().length > 0);

  const submit = () => {
    if (props.resolving || !complete()) return;
    props.onResolve(
      props.request,
      requestQuestions.map((item) => ({
        questionId: item.id,
        value: answers()[item.id] ?? "",
      })),
      "submit",
    );
  };
  const advance = () => {
    const current = question();
    if (!current || !(answers()[current.id] ?? "").trim()) return;
    if (questionIndex() < requestQuestions.length - 1) {
      setQuestionIndex((index) => index + 1);
    } else {
      submit();
    }
  };

  return (
    <article class="user-input-card agent-card approval" data-component="user-input-card">
      <header>
        <div class="user-input-card-title">
          <strong>{question()?.prompt}</strong>
        </div>
        <nav class="user-input-pager" aria-label={zh() ? "问题导航" : "Question navigation"}>
          <Button
            size="small"
            variant="ghost"
            aria-label={zh() ? "上一个问题" : "Previous question"}
            disabled={questionIndex() === 0 || props.resolving}
            onClick={() => setQuestionIndex((index) => Math.max(0, index - 1))}
          >
            <ChevronLeft size={15} />
          </Button>
          <small>
            {questionIndex() + 1}/{requestQuestions.length}
          </small>
          <Button
            size="small"
            variant="ghost"
            aria-label={zh() ? "下一个问题" : "Next question"}
            disabled={questionIndex() >= requestQuestions.length - 1 || props.resolving}
            onClick={advance}
          >
            <ChevronRight size={15} />
          </Button>
          <Button
            size="small"
            variant="ghost"
            aria-label={zh() ? "关闭" : "Close"}
            title={zh() ? "关闭" : "Close"}
            data-testid="workbench-cancel-user-input"
            disabled={props.resolving}
            onClick={() => props.onResolve(props.request, [], "cancel")}
          >
            <X size={15} />
          </Button>
        </nav>
      </header>
      <Show when={question()} keyed>
        {(current) => (
          <div class="user-input-question">
            <div class="user-input-choices" role="radiogroup" aria-label={current.header}>
              <For each={current.options}>
                {(option, index) => (
                  <Button
                    type="button"
                    classList={{
                      selected: !other()[current.id] && answers()[current.id] === option.value,
                    }}
                    role="radio"
                    aria-checked={!other()[current.id] && answers()[current.id] === option.value}
                    disabled={props.resolving}
                    onClick={() => {
                      setOther((value) => ({ ...value, [current.id]: false }));
                      setAnswers((value) => ({ ...value, [current.id]: option.value }));
                      if (questionIndex() < requestQuestions.length - 1) {
                        setQuestionIndex((value) => value + 1);
                      } else {
                        submit();
                      }
                    }}
                  >
                    <span class="choice-index">{index() + 1}</span>
                    <span>
                      <strong>{option.label}</strong>
                      <Show when={option.description}>
                        {(description) => <small>{description()}</small>}
                      </Show>
                    </span>
                  </Button>
                )}
              </For>
            </div>
            <div classList={{ "user-input-other-row": true, selected: other()[current.id] }}>
              <span class="choice-index" aria-hidden="true">
                <Pencil size={14} />
              </span>
              <TextField
                label={zh() ? "其他回答" : "Other answer"}
                hideLabel
                type={current.secret ? "password" : "text"}
                value={otherAnswers()[current.id] ?? ""}
                placeholder={
                  zh()
                    ? "否，并告诉 Agent 应该如何做得不同"
                    : "Other, tell the agent what to do differently"
                }
                disabled={props.resolving}
                onFocus={() => {
                  setOther((value) => ({ ...value, [current.id]: true }));
                  setAnswers((value) => ({
                    ...value,
                    [current.id]: otherAnswers()[current.id] ?? "",
                  }));
                }}
                onInput={(event) => {
                  const value = event.currentTarget.value;
                  setOther((state) => ({ ...state, [current.id]: true }));
                  setOtherAnswers((state) => ({ ...state, [current.id]: value }));
                  setAnswers((state) => ({ ...state, [current.id]: value }));
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.shiftKey) {
                    event.preventDefault();
                    advance();
                  }
                }}
              />
            </div>
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
          {zh() ? "跳过" : "Skip"}
        </Button>
      </footer>
    </article>
  );
}
