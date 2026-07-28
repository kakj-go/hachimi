import {
  commandFailure,
  commands,
  type McpInventorySnapshot,
  type McpPrompt,
  type McpPromptResult,
  type McpResource,
  type McpResourceContent,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, StatusBanner, TextField } from "@hachimi/ui";
import { For, Show, createSignal, type JSX } from "solid-js";

export function McpInventoryPanel(props: {
  snapshot: McpInventorySnapshot | undefined;
  loading: boolean;
  connectorEnabled: boolean;
  runtimeReady: boolean;
  onRefresh: () => Promise<void>;
}) {
  const i18n = useI18n();
  const copy = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const [resourcePreview, setResourcePreview] = createSignal<{
    uri: string;
    contents: McpResourceContent[];
  }>();
  const [promptPreview, setPromptPreview] = createSignal<{
    name: string;
    result: McpPromptResult;
  }>();
  const [promptArguments, setPromptArguments] = createSignal<Record<string, string>>({});
  const [busyKey, setBusyKey] = createSignal<string>();
  const [error, setError] = createSignal<string>();

  async function readResource(resource: McpResource) {
    const snapshot = props.snapshot;
    if (!snapshot) return;
    setBusyKey(`resource:${resource.uri}`);
    setError();
    try {
      setResourcePreview({
        uri: resource.uri,
        contents: await commands.readMcpResource({
          serverId: snapshot.serverId,
          uri: resource.uri,
        }),
      });
    } catch (reason) {
      setError(commandFailure(reason).message);
    } finally {
      setBusyKey();
    }
  }

  async function getPrompt(prompt: McpPrompt) {
    const snapshot = props.snapshot;
    if (!snapshot) return;
    setBusyKey(`prompt:${prompt.name}`);
    setError();
    try {
      const argumentsForPrompt = Object.fromEntries(
        prompt.arguments
          .map((argument) => [
            argument.name,
            promptArguments()[promptArgumentKey(prompt, argument.name)] ?? "",
          ])
          .filter(([, value]) => value !== ""),
      );
      setPromptPreview({
        name: prompt.name,
        result: await commands.getMcpPrompt({
          serverId: snapshot.serverId,
          name: prompt.name,
          arguments: argumentsForPrompt,
        }),
      });
    } catch (reason) {
      setError(commandFailure(reason).message);
    } finally {
      setBusyKey();
    }
  }

  return (
    <section class="mcp-inventory" data-testid="mcp-inventory">
      <div class="mcp-inventory-heading">
        <div>
          <h2>{copy("Resources 与 Prompts", "Resources and Prompts")}</h2>
          <span>
            {copy(
              "来自 MCP 服务的不受信任内容只用于预览或注入当前 Run，不能授予权限。",
              "Untrusted MCP content is only previewed or injected into the current Run; it cannot grant permissions.",
            )}
          </span>
        </div>
        <Button
          size="small"
          disabled={props.loading || !props.connectorEnabled}
          onClick={() => void props.onRefresh()}
        >
          {props.loading ? copy("刷新中…", "Refreshing…") : copy("刷新清单", "Refresh inventory")}
        </Button>
      </div>
      <Show when={error()}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <Show when={props.snapshot?.stale}>
        <StatusBanner tone="warning">
          {copy("当前显示上次验证的缓存。", "Showing the last verified cached inventory.")}
          <Show when={Object.keys(props.snapshot?.errors ?? {}).length > 0}>
            {` ${Object.entries(props.snapshot?.errors ?? {})
              .map(([surface, code]) => `${surface}: ${code}`)
              .join(", ")}`}
          </Show>
        </StatusBanner>
      </Show>
      <div class="mcp-inventory-grid">
        <InventoryGroup
          title={`Resources · ${props.snapshot?.resources.length ?? 0}`}
          empty={copy("服务没有声明 Resources。", "The server declares no Resources.")}
          count={props.snapshot?.resources.length ?? 0}
        >
          <For each={props.snapshot?.resources ?? []}>
            {(resource) => (
              <article class="mcp-inventory-card" data-testid={`mcp-resource-${resource.name}`}>
                <div>
                  <strong>{resource.title ?? resource.name}</strong>
                  <code>{resource.uri}</code>
                  <span>
                    {resource.description ?? resource.mimeType ?? copy("无描述", "No description")}
                  </span>
                </div>
                <Button
                  size="small"
                  disabled={
                    !props.connectorEnabled ||
                    !props.runtimeReady ||
                    busyKey() === `resource:${resource.uri}`
                  }
                  onClick={() => void readResource(resource)}
                >
                  {copy("读取", "Read")}
                </Button>
              </article>
            )}
          </For>
        </InventoryGroup>
        <InventoryGroup
          title={`Templates · ${props.snapshot?.resourceTemplates.length ?? 0}`}
          empty={copy(
            "服务没有声明 Resource Templates。",
            "The server declares no Resource Templates.",
          )}
          count={props.snapshot?.resourceTemplates.length ?? 0}
        >
          <For each={props.snapshot?.resourceTemplates ?? []}>
            {(template) => (
              <article class="mcp-inventory-card">
                <div>
                  <strong>{template.title ?? template.name}</strong>
                  <code>{template.uriTemplate}</code>
                  <span>
                    {template.description ?? template.mimeType ?? copy("无描述", "No description")}
                  </span>
                </div>
              </article>
            )}
          </For>
        </InventoryGroup>
      </div>
      <InventoryGroup
        title={`Prompts · ${props.snapshot?.prompts.length ?? 0}`}
        empty={copy("服务没有声明 Prompts。", "The server declares no Prompts.")}
        count={props.snapshot?.prompts.length ?? 0}
      >
        <For each={props.snapshot?.prompts ?? []}>
          {(prompt) => (
            <article class="mcp-prompt-card" data-testid={`mcp-prompt-${prompt.name}`}>
              <div class="mcp-prompt-copy">
                <strong>{prompt.title ?? prompt.name}</strong>
                <span>{prompt.description ?? copy("无描述", "No description")}</span>
              </div>
              <Show when={prompt.arguments.length > 0}>
                <div class="mcp-prompt-arguments">
                  <For each={prompt.arguments}>
                    {(argument) => (
                      <TextField
                        label={`${argument.name}${argument.required ? " *" : ""}`}
                        value={promptArguments()[promptArgumentKey(prompt, argument.name)] ?? ""}
                        placeholder={argument.description ?? ""}
                        onInput={(event) =>
                          setPromptArguments((current) => ({
                            ...current,
                            [promptArgumentKey(prompt, argument.name)]: event.currentTarget.value,
                          }))
                        }
                      />
                    )}
                  </For>
                </div>
              </Show>
              <Button
                size="small"
                disabled={
                  !props.connectorEnabled ||
                  !props.runtimeReady ||
                  busyKey() === `prompt:${prompt.name}` ||
                  prompt.arguments.some(
                    (argument) =>
                      argument.required &&
                      !(promptArguments()[promptArgumentKey(prompt, argument.name)] ?? "").trim(),
                  )
                }
                onClick={() => void getPrompt(prompt)}
              >
                {copy("获取 Prompt", "Get Prompt")}
              </Button>
            </article>
          )}
        </For>
      </InventoryGroup>
      <Show when={resourcePreview()}>
        {(preview) => (
          <details class="mcp-inventory-preview" open>
            <summary>
              {copy("Resource 预览", "Resource preview")} · {preview().uri}
            </summary>
            <For each={preview().contents}>
              {(content) => (
                <pre>
                  {content.text ??
                    copy(
                      `二进制内容 ${content.blobBase64?.length ?? 0} 个 base64 字符`,
                      `Binary content: ${content.blobBase64?.length ?? 0} base64 characters`,
                    )}
                </pre>
              )}
            </For>
          </details>
        )}
      </Show>
      <Show when={promptPreview()}>
        {(preview) => (
          <details class="mcp-inventory-preview" open>
            <summary>
              {copy("Prompt 预览", "Prompt preview")} · {preview().name}
            </summary>
            <pre>{JSON.stringify(preview().result, null, 2)}</pre>
          </details>
        )}
      </Show>
    </section>
  );
}

function InventoryGroup(props: {
  title: string;
  empty: string;
  count: number;
  children: JSX.Element;
}) {
  return (
    <section class="mcp-inventory-group">
      <h3>{props.title}</h3>
      <div class="mcp-inventory-list">
        {props.children}
        <Show when={props.count === 0}>
          <div class="extension-empty">{props.empty}</div>
        </Show>
      </div>
    </section>
  );
}

function promptArgumentKey(prompt: McpPrompt, argument: string) {
  return `${prompt.name}\0${argument}`;
}
