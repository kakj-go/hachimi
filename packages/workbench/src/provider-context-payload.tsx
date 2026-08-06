import type { ItemPayload } from "@hachimi/contracts";
import type { AppLocale } from "@hachimi/i18n";
import { Match, Show, Switch } from "solid-js";

type ProviderContextPayloadProps = {
  payload: ItemPayload;
  locale: AppLocale;
  text: string;
  focusable?: boolean;
};

export function ProviderContextPayload(props: ProviderContextPayloadProps) {
  const compaction = () =>
    props.payload.type === "context_compaction" ? props.payload.data : undefined;
  const reasoning = () => (props.payload.type === "reasoning" ? props.payload.data : undefined);
  return (
    <Switch fallback={<pre tabIndex={props.focusable ? 0 : undefined}>{props.text}</pre>}>
      <Match when={compaction()}>
        {(data) => (
          <article class="provider-context-payload" data-source={data().summary_source}>
            <header>
              <strong>{props.locale === "zh-CN" ? "上下文压缩" : "Context compaction"}</strong>
              <span
                class={
                  data().summary_source === "local_fallback"
                    ? "provider-source is-fallback"
                    : "provider-source"
                }
              >
                {compactionSource(data().summary_source, props.locale)}
              </span>
            </header>
            <dl>
              <div>
                <dt>{props.locale === "zh-CN" ? "实现" : "Implementation"}</dt>
                <dd>{data().implementation}</dd>
              </div>
              <Show when={data().provider_endpoint_id}>
                <div>
                  <dt>Provider</dt>
                  <dd>{data().provider_endpoint_id}</dd>
                </div>
              </Show>
              <Show when={data().fallback_reason}>
                <div>
                  <dt>{props.locale === "zh-CN" ? "降级原因" : "Fallback reason"}</dt>
                  <dd>{data().fallback_reason}</dd>
                </div>
              </Show>
            </dl>
            <Show when={data().warnings.length > 0}>
              <small>{data().warnings.join(" · ")}</small>
            </Show>
          </article>
        )}
      </Match>
      <Match when={reasoning()}>
        {(data) => (
          <article class="provider-context-payload" data-source={data().source}>
            <header>
              <strong>
                {props.locale === "zh-CN" ? "Provider 可展示摘要" : "Provider-visible summary"}
              </strong>
              <span class="provider-source">{data().source}</span>
            </header>
            <Show when={data().provider_endpoint_id}>
              <small>Provider: {data().provider_endpoint_id}</small>
            </Show>
            <pre>{props.text}</pre>
          </article>
        )}
      </Match>
    </Switch>
  );
}

function compactionSource(source: string, locale: AppLocale): string {
  if (locale !== "zh-CN") return source.replaceAll("_", " ");
  if (source === "provider_remote") return "Provider 远程压缩";
  if (source === "local_fallback") return "本地降级";
  return "本地压缩";
}
