import { For, Show, createEffect, createMemo, createSignal, type JSX } from "solid-js";
import { Search, SlidersHorizontal, Trash2, X } from "lucide-solid";
import { Button } from "./button";
import { Dialog } from "./dialog";
import { FormField, SegmentedControl, TextField } from "./forms";
import { AlertBanner, Checkbox } from "./patterns";

export type PermissionLevel = "read_only" | "writable" | "full_access";
export type FileAccess = "read" | "write" | "deny";

export interface PermissionFileRule {
  access: FileAccess;
  roots: string[];
  globs: string[];
  files?: string[];
  specialRoots: string[];
}

export interface PermissionCommandCandidate {
  name: string;
  executablePath: string;
  source: string;
}

export interface PermissionApplicationCandidate {
  identityHash: string;
  displayName: string;
  executableName: string;
  executablePath: string | null;
  iconPngBase64: string | null;
  windowCount: number;
}

export interface PermissionPolicyValue {
  level: PermissionLevel;
  revision: number;
  rules: {
    fileSystem: PermissionFileRule[];
    fileSystemUnrestrictedRead?: boolean;
    fileSystemUnrestrictedWrite?: boolean;
    network: {
      enabled: boolean;
      unrestrictedHosts?: boolean;
      hosts: string[];
      protocols: string[];
    };
    process: {
      spawn: boolean;
      interactive: boolean;
      unrestrictedCommands?: boolean;
      allowedCommands: string[];
    };
    browser: {
      observe?: boolean;
      act?: boolean;
      upload?: boolean;
      download?: boolean;
      cookieStorage?: boolean;
      cdp?: boolean;
      unrestrictedOrigins?: boolean;
      origins?: string[];
    };
    computer: {
      observe: boolean;
      act: boolean;
      unrestrictedTargets?: boolean;
      allowedApplications: string[];
      maxActions: number | null;
    };
    mcp: unknown[];
    connectors: unknown[];
  };
}

const splitValues = (value: string) => [
  ...new Set(
    value
      .split(/[\n,]+/)
      .map((item) => item.trim())
      .filter(Boolean),
  ),
];

function normalizeOrigin(value: string) {
  const trimmed = value.trim().replace(/\/$/, "");
  try {
    const url = new URL(trimmed);
    if (
      !["http:", "https:", "ws:", "wss:"].includes(url.protocol) ||
      !url.hostname ||
      url.username ||
      url.password ||
      url.pathname !== "/" ||
      url.search ||
      url.hash
    ) {
      return undefined;
    }
    return `${url.protocol}//${url.host}`;
  } catch {
    return undefined;
  }
}

function normalizeHost(value: string) {
  const host = value.trim().toLocaleLowerCase();
  if (!host || /[:/?#\s]/.test(host)) return undefined;
  const candidate = host.startsWith("*.") ? host.slice(2) : host;
  if (!candidate || candidate.startsWith(".") || candidate.endsWith(".")) return undefined;
  return /^[a-z0-9-]+(?:\.[a-z0-9-]+)*$/.test(candidate) ? host : undefined;
}

export function PermissionPolicyEditor(props: {
  value: PermissionPolicyValue;
  testId?: string;
  disabled?: boolean;
  zh: boolean;
  onChange: (value: PermissionPolicyValue) => void;
  chooseDirectory?: () => Promise<string | null>;
  chooseFiles?: (root: string) => Promise<string[]>;
  searchCommands?: (prefix: string) => Promise<PermissionCommandCandidate[]>;
  listApplications?: () => Promise<PermissionApplicationCandidate[]>;
  chooseForegroundApplication?: () => Promise<PermissionApplicationCandidate | null>;
}) {
  const [detailIndex, setDetailIndex] = createSignal<number>();
  const [commandDialogOpen, setCommandDialogOpen] = createSignal(false);
  const [commandPrefix, setCommandPrefix] = createSignal("");
  const [commandResults, setCommandResults] = createSignal<PermissionCommandCandidate[]>([]);
  const [selectedCommands, setSelectedCommands] = createSignal<string[]>([]);
  const [commandLoading, setCommandLoading] = createSignal(false);
  const [applicationDialogOpen, setApplicationDialogOpen] = createSignal(false);
  const [applications, setApplications] = createSignal<PermissionApplicationCandidate[]>([]);
  const [applicationQuery, setApplicationQuery] = createSignal("");
  const [selectedApplications, setSelectedApplications] = createSignal<string[]>([]);
  const [applicationLoading, setApplicationLoading] = createSignal(false);
  const [foregroundCapturePending, setForegroundCapturePending] = createSignal(false);
  const [listDrafts, setListDrafts] = createSignal<Record<string, string>>({});
  const [listErrors, setListErrors] = createSignal<Record<string, string>>({});

  const zh = () => props.zh;
  const disabled = () => Boolean(props.disabled);
  const rules = () => props.value.rules;
  const fileRules = (access: FileAccess) =>
    rules()
      .fileSystem.map((rule, index) => ({ rule, index }))
      .filter((item) => item.rule.access === access);
  const filteredApplications = createMemo(() => {
    const query = applicationQuery().trim().toLocaleLowerCase();
    return applications().filter(
      (candidate) =>
        !query ||
        `${candidate.displayName} ${candidate.executableName} ${candidate.executablePath ?? ""}`
          .toLocaleLowerCase()
          .includes(query),
    );
  });
  const unrestrictedScopes = createMemo(() => [
    Boolean(rules().fileSystemUnrestrictedRead),
    Boolean(rules().fileSystemUnrestrictedWrite),
    Boolean(rules().process.unrestrictedCommands),
    Boolean(rules().network.unrestrictedHosts),
    Boolean(rules().browser.unrestrictedOrigins),
    Boolean(rules().computer.unrestrictedTargets),
  ]);
  const equivalentToFullAccess = createMemo(
    () =>
      unrestrictedScopes().every(Boolean) &&
      rules().process.spawn &&
      rules().process.interactive &&
      rules().network.enabled &&
      rules().browser.observe &&
      rules().browser.act &&
      rules().browser.upload &&
      rules().browser.download &&
      rules().browser.cookieStorage &&
      rules().browser.cdp &&
      rules().computer.observe &&
      rules().computer.act,
  );

  const updateRules = (patch: Partial<PermissionPolicyValue["rules"]>) =>
    props.onChange({
      ...props.value,
      rules: { ...props.value.rules, ...patch },
    });

  function updateFileRule(index: number, patch: Partial<PermissionFileRule>) {
    const fileSystem = rules().fileSystem.map((rule, current) =>
      current === index ? { ...rule, ...patch } : rule,
    );
    updateRules({ fileSystem });
  }

  async function addDirectory(access: FileAccess) {
    if (!props.chooseDirectory || disabled()) return;
    const selected = await props.chooseDirectory();
    if (!selected) return;
    const exists = rules().fileSystem.some(
      (rule) =>
        rule.access === access &&
        rule.roots.some((root) => root.toLocaleLowerCase() === selected.toLocaleLowerCase()),
    );
    if (exists) return;
    updateRules({
      fileSystem: [
        ...rules().fileSystem,
        { access, roots: [selected], globs: [], files: [], specialRoots: [] },
      ],
    });
  }

  function removeFileRule(index: number) {
    updateRules({ fileSystem: rules().fileSystem.filter((_, current) => current !== index) });
    if (detailIndex() === index) setDetailIndex(undefined);
  }

  async function runCommandSearch() {
    if (!props.searchCommands || !commandPrefix().trim()) return;
    setCommandLoading(true);
    try {
      setCommandResults(await props.searchCommands(commandPrefix().trim()));
      setSelectedCommands([]);
    } finally {
      setCommandLoading(false);
    }
  }

  function toggleSelected(list: string[], value: string) {
    return list.includes(value) ? list.filter((item) => item !== value) : [...list, value];
  }

  function addSelectedCommands() {
    const current = new Set(rules().process.allowedCommands);
    selectedCommands().forEach((path) => current.add(path));
    updateRules({ process: { ...rules().process, allowedCommands: [...current] } });
    setCommandDialogOpen(false);
  }

  async function refreshApplications() {
    if (!props.listApplications || disabled()) return;
    setApplicationLoading(true);
    try {
      setApplications(await props.listApplications());
    } finally {
      setApplicationLoading(false);
    }
  }

  async function openApplications() {
    if (!props.listApplications || disabled()) return;
    setApplicationDialogOpen(true);
    setSelectedApplications([]);
    await refreshApplications();
  }

  async function addForegroundApplication() {
    if (!props.chooseForegroundApplication || disabled()) return;
    setForegroundCapturePending(true);
    try {
      const candidate = await props.chooseForegroundApplication();
      if (!candidate) return;
      const current = new Set(rules().computer.allowedApplications);
      current.add(candidate.identityHash);
      updateRules({ computer: { ...rules().computer, allowedApplications: [...current] } });
    } finally {
      setForegroundCapturePending(false);
    }
  }

  function addSelectedApplications() {
    const current = new Set(rules().computer.allowedApplications);
    selectedApplications().forEach((identityHash) => current.add(identityHash));
    updateRules({ computer: { ...rules().computer, allowedApplications: [...current] } });
    setApplicationDialogOpen(false);
  }

  function listDraft(key: string) {
    return listDrafts()[key] ?? "";
  }

  function setListDraft(key: string, value: string) {
    setListDrafts((current) => ({ ...current, [key]: value }));
  }

  function addListValue(
    key: string,
    value: string,
    normalize?: (value: string) => string | undefined,
  ) {
    const normalized = normalize ? normalize(value) : value.trim();
    if (!normalized) {
      setListErrors((current) => ({
        ...current,
        [key]:
          key === "hosts"
            ? zh()
              ? "请输入域名或 *.example.com，不要包含协议、路径或端口。"
              : "Enter a host or *.example.com without a protocol, path, or port."
            : zh()
              ? "请输入仅包含协议、主机和可选端口的 Origin。"
              : "Enter an Origin containing only a protocol, host, and optional port.",
      }));
      return;
    }
    setListErrors((current) => ({ ...current, [key]: "" }));
    const current = key === "hosts" ? rules().network.hosts : (rules().browser.origins ?? []);
    if (current.some((item) => item.toLocaleLowerCase() === normalized.toLocaleLowerCase())) return;
    if (key === "hosts") {
      updateRules({
        network: { ...rules().network, enabled: true, hosts: [...current, normalized] },
      });
    } else {
      updateRules({ browser: { ...rules().browser, origins: [...current, normalized] } });
    }
    setListDraft(key, "");
  }

  function removeListValue(key: string, value: string) {
    if (key === "hosts") {
      const hosts = rules().network.hosts.filter((item) => item !== value);
      updateRules({
        network: {
          ...rules().network,
          enabled: Boolean(rules().network.unrestrictedHosts) || hosts.length > 0,
          hosts,
        },
      });
    } else {
      updateRules({
        browser: {
          ...rules().browser,
          origins: (rules().browser.origins ?? []).filter((item) => item !== value),
        },
      });
    }
  }

  return (
    <div data-component="permission-policy-editor">
      <FormField
        label={zh() ? "权限档位" : "Permission level"}
        description={
          zh()
            ? "后台运行超出预配置范围时进入需要处理。"
            : "Background runs move to Needs attention outside the configured scope."
        }
      >
        <SegmentedControl<PermissionLevel>
          label={zh() ? "权限档位" : "Permission level"}
          {...(props.testId ? { testId: props.testId } : {})}
          value={props.value.level}
          disabled={disabled()}
          options={[
            { value: "read_only", label: zh() ? "只读" : "Read only" },
            { value: "writable", label: zh() ? "可写" : "Writable" },
            { value: "full_access", label: zh() ? "完全授权" : "Full access" },
          ]}
          onChange={(level) => props.onChange({ ...props.value, level })}
        />
      </FormField>

      <Show when={props.value.level !== "full_access"}>
        <Show when={unrestrictedScopes().some(Boolean)}>
          <AlertBanner tone="danger">
            {equivalentToFullAccess()
              ? zh()
                ? "当前范围与能力组合等价于完全授权；系统安全边界仍然生效。"
                : "The current scope and capability combination is equivalent to Full access; system safety boundaries still apply."
              : zh()
                ? "部分资源范围已设为不限制，请确认这是预期授权。"
                : "Some resource scopes are unrestricted. Confirm that this authority is intentional."}
          </AlertBanner>
        </Show>
        <div data-component="permission-policy-grid">
          <section>
            <h4>{zh() ? "文件系统" : "File system"}</h4>
            <ScopeToggle
              label={zh() ? "所有目录可读" : "All directories readable"}
              checked={Boolean(rules().fileSystemUnrestrictedRead)}
              disabled={disabled()}
              onChange={(checked) => updateRules({ fileSystemUnrestrictedRead: checked })}
            />
            <DirectoryList
              title={zh() ? "只读目录" : "Read-only directories"}
              rules={fileRules("read")}
              zh={zh()}
              disabled={disabled()}
              onAdd={() => void addDirectory("read")}
              onRemove={removeFileRule}
              onDetail={setDetailIndex}
            />
            <Show when={props.value.level === "writable"}>
              <ScopeToggle
                label={zh() ? "所有目录可写" : "All directories writable"}
                checked={Boolean(rules().fileSystemUnrestrictedWrite)}
                disabled={disabled()}
                onChange={(checked) => updateRules({ fileSystemUnrestrictedWrite: checked })}
              />
              <DirectoryList
                title={zh() ? "可写目录" : "Writable directories"}
                rules={fileRules("write")}
                zh={zh()}
                disabled={disabled()}
                onAdd={() => void addDirectory("write")}
                onRemove={removeFileRule}
                onDetail={setDetailIndex}
              />
            </Show>
            <DirectoryList
              title={zh() ? "拒绝目录" : "Denied directories"}
              rules={fileRules("deny")}
              zh={zh()}
              disabled={disabled()}
              onAdd={() => void addDirectory("deny")}
              onRemove={removeFileRule}
              onDetail={setDetailIndex}
            />
          </section>

          <section>
            <h4>{zh() ? "进程与网络" : "Process and network"}</h4>
            <ScopeToggle
              label={zh() ? "允许沙箱进程" : "Allow sandboxed processes"}
              checked={rules().process.spawn}
              disabled={disabled() || props.value.level === "read_only"}
              onChange={(checked) =>
                updateRules({ process: { ...rules().process, spawn: checked } })
              }
            />
            <ScopeToggle
              label={zh() ? "允许交互进程" : "Allow interactive processes"}
              checked={rules().process.interactive}
              disabled={disabled() || props.value.level === "read_only"}
              onChange={(checked) =>
                updateRules({ process: { ...rules().process, interactive: checked } })
              }
            />
            <ScopeToggle
              label={zh() ? "所有命令" : "All commands"}
              checked={Boolean(rules().process.unrestrictedCommands)}
              disabled={disabled() || props.value.level === "read_only"}
              onChange={(checked) =>
                updateRules({ process: { ...rules().process, unrestrictedCommands: checked } })
              }
            />
            <TagList
              title={zh() ? "允许的命令" : "Allowed commands"}
              values={rules().process.allowedCommands}
              placeholder={zh() ? "点击添加命令" : "Add a command"}
              zh={zh()}
              disabled={disabled() || props.value.level === "read_only"}
              onRemove={(value) =>
                updateRules({
                  process: {
                    ...rules().process,
                    allowedCommands: rules().process.allowedCommands.filter(
                      (item) => item !== value,
                    ),
                  },
                })
              }
              onAdd={() => {
                setCommandDialogOpen(true);
                setCommandPrefix("");
                setCommandResults([]);
              }}
            />
            <ScopeToggle
              label={zh() ? "所有域名" : "All network hosts"}
              checked={Boolean(rules().network.unrestrictedHosts)}
              disabled={disabled()}
              onChange={(checked) =>
                updateRules({
                  network: {
                    ...rules().network,
                    unrestrictedHosts: checked,
                    enabled: checked || rules().network.hosts.length > 0,
                    protocols: checked ? ["http", "https", "ws", "wss"] : rules().network.protocols,
                  },
                })
              }
            />
            <TagList
              title={zh() ? "网络域名" : "Network hosts"}
              values={rules().network.hosts}
              placeholder={zh() ? "输入域名后按 Enter" : "Type a host and press Enter"}
              draft={listDraft("hosts")}
              onDraft={(value) => setListDraft("hosts", value)}
              onCommit={(value) => addListValue("hosts", value, normalizeHost)}
              onRemove={(value) => removeListValue("hosts", value)}
              onAdd={() => addListValue("hosts", listDraft("hosts"), normalizeHost)}
              error={listErrors().hosts}
              zh={zh()}
              disabled={disabled()}
            />
          </section>

          <section>
            <h4>Browser</h4>
            <div data-component="permission-policy-checks">
              <BrowserCheckbox
                field="observe"
                label={zh() ? "查看网页内容" : "View page content"}
                {...props}
              />
              <BrowserCheckbox
                field="act"
                label={zh() ? "操作网页" : "Interact with pages"}
                {...props}
              />
              <BrowserCheckbox
                field="upload"
                label={zh() ? "上传文件" : "Upload files"}
                {...props}
              />
              <BrowserCheckbox
                field="download"
                label={zh() ? "下载文件" : "Download files"}
                {...props}
              />
              <BrowserCheckbox
                field="cookieStorage"
                label={zh() ? "使用网站登录状态" : "Use website sign-in state"}
                description={
                  zh() ? "允许读取或保存网站登录状态。" : "Read and save website sign-in state."
                }
                {...props}
              />
              <BrowserCheckbox
                field="cdp"
                label={zh() ? "高级浏览器控制" : "Advanced browser control"}
                description={
                  zh()
                    ? "允许访问更底层的浏览器调试能力。"
                    : "Access lower-level browser debugging controls."
                }
                {...props}
              />
            </div>
            <ScopeToggle
              label={zh() ? "所有 Origin" : "All Origins"}
              checked={Boolean(rules().browser.unrestrictedOrigins)}
              disabled={disabled()}
              onChange={(checked) =>
                updateRules({ browser: { ...rules().browser, unrestrictedOrigins: checked } })
              }
            />
            <TagList
              title={zh() ? "允许的 Origin" : "Allowed origins"}
              values={rules().browser.origins ?? []}
              placeholder={zh() ? "输入 Origin 后按 Enter" : "Type an Origin and press Enter"}
              draft={listDraft("origins")}
              onDraft={(value) => setListDraft("origins", value)}
              onCommit={(value) => addListValue("origins", value, normalizeOrigin)}
              onRemove={(value) => removeListValue("origins", value)}
              onAdd={() => addListValue("origins", listDraft("origins"), normalizeOrigin)}
              error={listErrors().origins}
              zh={zh()}
              disabled={disabled()}
            />
          </section>

          <section>
            <h4>{zh() ? "桌面控制" : "Computer control"}</h4>
            <div data-component="permission-policy-checks">
              <ScopeToggle
                label={zh() ? "观察" : "Observe"}
                checked={rules().computer.observe}
                disabled={disabled()}
                onChange={(checked) =>
                  updateRules({ computer: { ...rules().computer, observe: checked } })
                }
              />
              <ScopeToggle
                label={zh() ? "操作" : "Act"}
                checked={rules().computer.act}
                disabled={disabled() || props.value.level === "read_only"}
                onChange={(checked) =>
                  updateRules({ computer: { ...rules().computer, act: checked } })
                }
              />
            </div>
            <ScopeToggle
              label={zh() ? "所有应用" : "All applications"}
              checked={Boolean(rules().computer.unrestrictedTargets)}
              disabled={disabled()}
              onChange={(checked) =>
                updateRules({ computer: { ...rules().computer, unrestrictedTargets: checked } })
              }
            />
            <TagList
              title={zh() ? "允许的应用" : "Allowed applications"}
              values={rules().computer.allowedApplications}
              placeholder={zh() ? "点击选择应用" : "Choose an application"}
              zh={zh()}
              disabled={disabled()}
              onRemove={(value) =>
                updateRules({
                  computer: {
                    ...rules().computer,
                    allowedApplications: rules().computer.allowedApplications.filter(
                      (item) => item !== value,
                    ),
                  },
                })
              }
              onAdd={() => void openApplications()}
              extraAction={
                props.chooseForegroundApplication ? (
                  <Button
                    variant="ghost"
                    size="small"
                    disabled={foregroundCapturePending()}
                    loading={foregroundCapturePending()}
                    onClick={() => void addForegroundApplication()}
                  >
                    {foregroundCapturePending()
                      ? zh()
                        ? "请在 3 秒内切换到目标窗口"
                        : "Switch to the target within 3 seconds"
                      : zh()
                        ? "从前台窗口添加"
                        : "Add foreground app"}
                  </Button>
                ) : undefined
              }
            />
            <TextField
              label={zh() ? "最大操作数" : "Maximum actions"}
              type="number"
              value={rules().computer.maxActions?.toString() ?? ""}
              disabled={disabled() || props.value.level === "read_only"}
              onInput={(event) => {
                const value = Number.parseInt(event.currentTarget.value, 10);
                updateRules({
                  computer: {
                    ...rules().computer,
                    maxActions: Number.isFinite(value) && value > 0 ? value : null,
                  },
                });
              }}
            />
          </section>
        </div>
      </Show>

      <Show when={detailIndex() !== undefined}>
        <FileDetailDialog
          rule={rules().fileSystem[detailIndex() ?? 0]}
          zh={zh()}
          disabled={disabled()}
          {...(props.chooseFiles ? { chooseFiles: props.chooseFiles } : {})}
          onClose={() => setDetailIndex(undefined)}
          onSave={(patch) => {
            const index = detailIndex();
            if (index !== undefined) updateFileRule(index, patch);
            setDetailIndex(undefined);
          }}
        />
      </Show>

      <CommandDialog
        open={commandDialogOpen()}
        title={zh() ? "添加允许的命令" : "Add allowed commands"}
        description={
          zh()
            ? "输入命令前缀后按 Tab 或点击搜索。"
            : "Type a command prefix, then press Tab or Search."
        }
        prefix={commandPrefix()}
        results={commandResults()}
        selected={selectedCommands()}
        loading={commandLoading()}
        zh={zh()}
        onPrefix={(value) => setCommandPrefix(value)}
        onSearch={() => void runCommandSearch()}
        onToggle={(value) => setSelectedCommands((current) => toggleSelected(current, value))}
        onClose={() => setCommandDialogOpen(false)}
        onAdd={addSelectedCommands}
      />

      <ApplicationDialog
        open={applicationDialogOpen()}
        applications={filteredApplications()}
        selected={selectedApplications()}
        query={applicationQuery()}
        loading={applicationLoading()}
        zh={zh()}
        onQuery={setApplicationQuery}
        onRefresh={() => void refreshApplications()}
        onToggle={(value) => setSelectedApplications((current) => toggleSelected(current, value))}
        onClose={() => setApplicationDialogOpen(false)}
        onAdd={addSelectedApplications}
      />
    </div>
  );
}

function ScopeToggle(props: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <Checkbox
      class="permission-policy-toggle"
      label={props.label}
      checked={props.checked}
      disabled={props.disabled}
      onChange={(event) => props.onChange(event.currentTarget.checked)}
    />
  );
}

function DirectoryList(props: {
  title: string;
  rules: { rule: PermissionFileRule; index: number }[];
  zh: boolean;
  disabled?: boolean;
  onAdd: () => void;
  onRemove: (index: number) => void;
  onDetail: (index: number) => void;
}) {
  return (
    <div class="permission-policy-list">
      <header>
        <strong>{props.title}</strong>
        <Button variant="ghost" size="small" disabled={props.disabled} onClick={props.onAdd}>
          + {props.zh ? "添加目录" : "Add directory"}
        </Button>
      </header>
      <Show
        when={props.rules.length > 0}
        fallback={
          <span class="permission-policy-empty">{props.zh ? "尚未添加" : "None added"}</span>
        }
      >
        <For each={props.rules}>
          {(entry) => (
            <div class="permission-policy-item">
              <span class="permission-policy-item-label" title={entry.rule.roots.join(", ")}>
                {entry.rule.roots.join(", ")}
              </span>
              <span class="permission-policy-item-meta">
                {entry.rule.files?.length || entry.rule.globs.length
                  ? props.zh
                    ? "已设置精细规则"
                    : "Fine rules configured"
                  : props.zh
                    ? "包含全部子目录和文件"
                    : "Includes descendants"}
              </span>
              <Button
                variant="ghost"
                size="small"
                disabled={props.disabled}
                title={props.zh ? "精细设置" : "Fine settings"}
                aria-label={props.zh ? "精细设置" : "Fine settings"}
                onClick={() => props.onDetail(entry.index)}
              >
                <SlidersHorizontal size={15} aria-hidden="true" />
              </Button>
              <Button
                variant="ghost"
                size="small"
                disabled={props.disabled}
                title={props.zh ? "删除目录" : "Remove directory"}
                aria-label={props.zh ? "删除目录" : "Remove directory"}
                onClick={() => props.onRemove(entry.index)}
              >
                <Trash2 size={15} aria-hidden="true" />
              </Button>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}

function TagList(props: {
  title: string;
  values: string[];
  placeholder: string;
  zh: boolean;
  disabled?: boolean;
  draft?: string;
  onDraft?: (value: string) => void;
  onCommit?: (value: string) => void;
  onRemove: (value: string) => void;
  onAdd: () => void;
  extraAction?: JSX.Element;
  error?: string | undefined;
}) {
  return (
    <div class="permission-policy-list">
      <header>
        <strong>{props.title}</strong>
        <div class="permission-policy-list-actions">
          {props.extraAction}
          <Button variant="ghost" size="small" disabled={props.disabled} onClick={props.onAdd}>
            + {props.zh ? "添加" : "Add"}
          </Button>
        </div>
      </header>
      <Show when={props.values.length > 0}>
        <div class="permission-policy-tags">
          <For each={props.values}>
            {(value) => (
              <span class="permission-policy-tag" title={value}>
                {value}
                <button
                  type="button"
                  aria-label={`${props.zh ? "删除" : "Remove"} ${value}`}
                  disabled={props.disabled}
                  onClick={() => props.onRemove(value)}
                >
                  <X size={13} aria-hidden="true" />
                </button>
              </span>
            )}
          </For>
        </div>
      </Show>
      <Show when={props.onDraft}>
        <div class="permission-policy-inline-input">
          <input
            class="ui-input"
            value={props.draft ?? ""}
            placeholder={props.placeholder}
            disabled={props.disabled}
            onInput={(event) => props.onDraft?.(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                props.onCommit?.(event.currentTarget.value);
              }
            }}
          />
        </div>
      </Show>
      <Show when={!props.values.length && !props.onDraft}>
        <span class="permission-policy-empty">{props.placeholder}</span>
      </Show>
      <Show when={props.error}>
        {(error) => <span class="permission-policy-validation">{error()}</span>}
      </Show>
    </div>
  );
}

function BrowserCheckbox(props: {
  field: "observe" | "act" | "upload" | "download" | "cookieStorage" | "cdp";
  label: string;
  description?: string;
  value: PermissionPolicyValue;
  disabled?: boolean;
  zh: boolean;
  onChange: (value: PermissionPolicyValue) => void;
}) {
  const allowed = () => Boolean(props.value.rules.browser[props.field]);
  return (
    <div class="permission-policy-browser-check">
      <Checkbox
        label={props.label}
        checked={allowed()}
        disabled={
          props.disabled || (props.value.level === "read_only" && props.field !== "observe")
        }
        onChange={(event) =>
          props.onChange({
            ...props.value,
            rules: {
              ...props.value.rules,
              browser: { ...props.value.rules.browser, [props.field]: event.currentTarget.checked },
            },
          })
        }
      />
      <Show when={props.description}>
        <span>{props.description}</span>
      </Show>
    </div>
  );
}

function FileDetailDialog(props: {
  rule: PermissionFileRule | undefined;
  zh: boolean;
  disabled?: boolean;
  chooseFiles?: (root: string) => Promise<string[]>;
  onClose: () => void;
  onSave: (patch: Partial<PermissionFileRule>) => void;
}) {
  const [globs, setGlobs] = createSignal("");
  const [files, setFiles] = createSignal<string[]>([]);
  createEffect(() => {
    const rule = props.rule;
    setGlobs(rule?.globs.join("\n") ?? "");
    setFiles(rule?.files ?? []);
  });
  async function chooseFiles() {
    const root = props.rule?.roots[0];
    if (!root || !props.chooseFiles) return;
    const selected = await props.chooseFiles(root);
    setFiles((current) => [...new Set([...current, ...selected])]);
  }
  return (
    <Dialog
      open={Boolean(props.rule)}
      title={props.zh ? "目录精细设置" : "Directory fine settings"}
      description={
        props.zh
          ? "目录默认递归继承权限；这里可以进一步限制到 Glob 或具体文件。"
          : "The directory is recursive by default; narrow it to globs or exact files here."
      }
      size="wide"
      closeLabel={props.zh ? "关闭" : "Close"}
      onOpenChange={(open) => !open && props.onClose()}
    >
      <div class="permission-policy-detail-dialog">
        <code>{props.rule?.roots.join(", ")}</code>
        <TextField
          label={props.zh ? "Glob 表达式" : "Glob expressions"}
          value={globs()}
          disabled={Boolean(props.disabled)}
          placeholder="src/**/*.ts"
          onInput={(event) => setGlobs(event.currentTarget.value)}
        />
        <div class="permission-policy-file-selection">
          <header>
            <strong>{props.zh ? "精确文件" : "Exact files"}</strong>
            <Button
              variant="ghost"
              size="small"
              disabled={props.disabled || !props.chooseFiles}
              onClick={() => void chooseFiles()}
            >
              {props.zh ? "选择文件" : "Choose files"}
            </Button>
          </header>
          <For
            each={files()}
            fallback={
              <span class="permission-policy-empty">
                {props.zh ? "未选择具体文件" : "No exact files"}
              </span>
            }
          >
            {(file) => (
              <div class="permission-policy-item">
                <code>{file}</code>
                <Button
                  variant="ghost"
                  size="small"
                  onClick={() => setFiles((current) => current.filter((item) => item !== file))}
                >
                  <Trash2 size={15} aria-hidden="true" />
                </Button>
              </div>
            )}
          </For>
        </div>
        <div class="dialog-actions">
          <Button variant="ghost" onClick={props.onClose}>
            {props.zh ? "取消" : "Cancel"}
          </Button>
          <Button
            variant="primary"
            onClick={() => props.onSave({ globs: splitValues(globs()), files: files() })}
          >
            {props.zh ? "保存规则" : "Save rules"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function CommandDialog(props: {
  open: boolean;
  title: string;
  description: string;
  prefix: string;
  results: PermissionCommandCandidate[];
  selected: string[];
  loading: boolean;
  zh: boolean;
  onPrefix: (value: string) => void;
  onSearch: () => void;
  onToggle: (value: string) => void;
  onClose: () => void;
  onAdd: () => void;
}) {
  const [activeResult, setActiveResult] = createSignal(0);
  return (
    <Dialog
      open={props.open}
      title={props.title}
      description={props.description}
      size="wide"
      closeLabel={props.zh ? "关闭" : "Close"}
      onOpenChange={(open) => !open && props.onClose()}
    >
      <div class="permission-policy-search-dialog">
        <div class="permission-policy-search-row">
          <input
            class="ui-input"
            autofocus
            value={props.prefix}
            placeholder={props.zh ? "输入命令前缀，例如 git" : "Command prefix, e.g. git"}
            onInput={(event) => props.onPrefix(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Tab") {
                event.preventDefault();
                props.onSearch();
              } else if (event.key === "ArrowDown" && props.results.length) {
                event.preventDefault();
                setActiveResult((current) => Math.min(props.results.length - 1, current + 1));
              } else if (event.key === "ArrowUp" && props.results.length) {
                event.preventDefault();
                setActiveResult((current) => Math.max(0, current - 1));
              } else if (event.key === "Enter") {
                event.preventDefault();
                const candidate = props.results[activeResult()];
                if (!candidate) props.onSearch();
                else if (props.selected.includes(candidate.executablePath)) props.onAdd();
                else props.onToggle(candidate.executablePath);
              }
            }}
          />
          <Button
            variant="ghost"
            aria-label={props.zh ? "搜索命令" : "Search commands"}
            title={props.zh ? "搜索命令" : "Search commands"}
            loading={props.loading}
            onClick={props.onSearch}
          >
            <Search size={17} aria-hidden="true" />
          </Button>
        </div>
        <div class="permission-policy-search-results">
          <For
            each={props.results}
            fallback={
              <span class="permission-policy-empty">
                {props.zh ? "输入前缀后搜索命令" : "Search for command candidates"}
              </span>
            }
          >
            {(candidate, index) => (
              <div classList={{ "permission-policy-result-active": index() === activeResult() }}>
                <Checkbox
                  label={`${candidate.name} · ${candidate.executablePath} · ${candidate.source}`}
                  checked={props.selected.includes(candidate.executablePath)}
                  onChange={() => props.onToggle(candidate.executablePath)}
                />
              </div>
            )}
          </For>
        </div>
        <div class="dialog-actions">
          <Button variant="ghost" onClick={props.onClose}>
            {props.zh ? "取消" : "Cancel"}
          </Button>
          <Button variant="primary" disabled={!props.selected.length} onClick={props.onAdd}>
            {props.zh ? "添加选中命令" : "Add selected"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function ApplicationDialog(props: {
  open: boolean;
  applications: PermissionApplicationCandidate[];
  selected: string[];
  query: string;
  loading: boolean;
  zh: boolean;
  onQuery: (value: string) => void;
  onRefresh: () => void;
  onToggle: (value: string) => void;
  onClose: () => void;
  onAdd: () => void;
}) {
  return (
    <Dialog
      open={props.open}
      title={props.zh ? "选择允许的应用" : "Choose allowed applications"}
      description={
        props.zh
          ? "选择应用身份，不保存易失的窗口句柄。"
          : "Choose stable application identities; volatile window handles are not persisted."
      }
      size="wide"
      closeLabel={props.zh ? "关闭" : "Close"}
      onOpenChange={(open) => !open && props.onClose()}
    >
      <div class="permission-policy-search-dialog">
        <div class="permission-policy-search-row">
          <input
            class="ui-input"
            value={props.query}
            placeholder={props.zh ? "搜索应用名称或可执行文件" : "Search apps or executables"}
            onInput={(event) => props.onQuery(event.currentTarget.value)}
          />
          <Button variant="ghost" disabled={props.loading} onClick={props.onRefresh}>
            {props.zh ? "刷新" : "Refresh"}
          </Button>
        </div>
        <div class="permission-policy-search-results">
          <For
            each={props.applications}
            fallback={
              <span class="permission-policy-empty">
                {props.loading
                  ? props.zh
                    ? "正在读取应用"
                    : "Loading applications"
                  : props.zh
                    ? "没有可用应用"
                    : "No applications"}
              </span>
            }
          >
            {(candidate) => (
              <div class="permission-policy-application-option">
                <Show
                  when={candidate.iconPngBase64}
                  fallback={
                    <span class="permission-policy-application-fallback">
                      {candidate.displayName.slice(0, 1).toUpperCase()}
                    </span>
                  }
                >
                  <img src={`data:image/png;base64,${candidate.iconPngBase64}`} alt="" />
                </Show>
                <Checkbox
                  label={`${candidate.displayName} (${candidate.executableName})${candidate.windowCount ? ` · ${candidate.windowCount} ${props.zh ? "个窗口" : "windows"}` : ""}`}
                  checked={props.selected.includes(candidate.identityHash)}
                  onChange={() => props.onToggle(candidate.identityHash)}
                />
              </div>
            )}
          </For>
        </div>
        <div class="dialog-actions">
          <Button variant="ghost" onClick={props.onClose}>
            {props.zh ? "取消" : "Cancel"}
          </Button>
          <Button variant="primary" disabled={!props.selected.length} onClick={props.onAdd}>
            {props.zh ? "添加选中应用" : "Add selected"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
