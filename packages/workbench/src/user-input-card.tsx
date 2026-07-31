import type {
  UserInputAnswer,
  UserInputRequestRecord,
  UserInputResolutionAction,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, MessageCircle, SelectField, TextField } from "@hachimi/ui";
import { For, Show, createSignal, untrack } from "solid-js";

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
        </span>
      </header>
      <For each={props.request.questions}>
        {(question) => (
          <div class="user-input-question">
            <Show when={question.options.length > 0}>
              <SelectField
                label={question.header}
                description={question.prompt}
                value={answers()[question.id] ?? ""}
                options={[
                  ...question.options.map((option) => ({
                    value: option.value,
                    label: option.label,
                  })),
                  {
                    value: "",
                    label: i18n.locale() === "zh-CN" ? "自由输入…" : "Free-form answer…",
                  },
                ]}
                onChange={(value) =>
                  setAnswers((current) => ({
                    ...current,
                    [question.id]: value,
                  }))
                }
              />
            </Show>
            <TextField
              label={question.options.length > 0 ? question.prompt : question.header}
              {...(question.options.length > 0 ? {} : { description: question.prompt })}
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
          </div>
        )}
      </For>
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
