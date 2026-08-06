import { For, Show } from "solid-js";
import { FormField, SegmentedControl, TextField } from "./forms";
import { Checkbox } from "./patterns";

export type PermissionLevel = "read_only" | "writable" | "full_access";
export type FileAccess = "read" | "write" | "deny";

export interface PermissionFileRule {
  access: FileAccess;
  roots: string[];
  globs: string[];
  specialRoots: string[];
}

export interface PermissionPolicyValue {
  level: PermissionLevel;
  revision: number;
  rules: {
    fileSystem: PermissionFileRule[];
    network: { enabled: boolean; hosts: string[]; protocols: string[] };
    process: { spawn: boolean; interactive: boolean; allowedCommands: string[] };
    browser: {
      observe?: boolean;
      act?: boolean;
      upload?: boolean;
      download?: boolean;
      cookieStorage?: boolean;
      cdp?: boolean;
      origins?: string[];
    };
    computer: {
      observe: boolean;
      act: boolean;
      targetWindows: string[];
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

export function PermissionPolicyEditor(props: {
  value: PermissionPolicyValue;
  testId?: string;
  disabled?: boolean;
  zh: boolean;
  onChange: (value: PermissionPolicyValue) => void;
}) {
  const updateRules = (patch: Partial<PermissionPolicyValue["rules"]>) =>
    props.onChange({
      ...props.value,
      rules: { ...props.value.rules, ...patch },
    });
  const fileValues = (access: FileAccess, field: "roots" | "globs" | "specialRoots") =>
    props.value.rules.fileSystem
      .filter((grant) => grant.access === access)
      .flatMap((grant) => grant[field]);
  const updateFileValues = (
    access: FileAccess,
    field: "roots" | "globs" | "specialRoots",
    value: string,
  ) => {
    const existing = props.value.rules.fileSystem.filter((grant) => grant.access === access);
    const retained = props.value.rules.fileSystem.filter((grant) => grant.access !== access);
    const merged: PermissionFileRule = existing.reduce(
      (rule, current) => ({
        access,
        roots: [...rule.roots, ...current.roots],
        globs: [...rule.globs, ...current.globs],
        specialRoots: [...rule.specialRoots, ...current.specialRoots],
      }),
      { access, roots: [], globs: [], specialRoots: [] },
    );
    merged[field] = splitValues(value);
    const hasScope = merged.roots.length || merged.globs.length || merged.specialRoots.length;
    updateRules({ fileSystem: hasScope ? [...retained, merged] : retained });
  };

  return (
    <div data-component="permission-policy-editor">
      <FormField
        label={props.zh ? "权限档位" : "Permission level"}
        description={
          props.zh
            ? "后台运行超出预配置范围时进入需要处理。"
            : "Background runs move to Needs attention outside the configured scope."
        }
      >
        <SegmentedControl<PermissionLevel>
          label={props.zh ? "权限档位" : "Permission level"}
          {...(props.testId ? { testId: props.testId } : {})}
          value={props.value.level}
          disabled={Boolean(props.disabled)}
          options={[
            { value: "read_only", label: props.zh ? "只读" : "Read only" },
            { value: "writable", label: props.zh ? "可写" : "Writable" },
            { value: "full_access", label: props.zh ? "完全授权" : "Full access" },
          ]}
          onChange={(level) => props.onChange({ ...props.value, level })}
        />
      </FormField>

      <Show when={props.value.level !== "full_access"}>
        <div data-component="permission-policy-grid">
          <section>
            <h4>{props.zh ? "文件系统" : "File system"}</h4>
            <TextField
              label={props.zh ? "额外只读目录" : "Additional read roots"}
              value={fileValues("read", "roots").join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) => updateFileValues("read", "roots", event.currentTarget.value)}
            />
            <TextField
              label={props.zh ? "只读 Glob" : "Read globs"}
              value={fileValues("read", "globs").join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) => updateFileValues("read", "globs", event.currentTarget.value)}
            />
            <TextField
              label={props.zh ? "只读特殊根" : "Read special roots"}
              value={fileValues("read", "specialRoots").join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) =>
                updateFileValues("read", "specialRoots", event.currentTarget.value)
              }
            />
            <Show when={props.value.level === "writable"}>
              <TextField
                label={props.zh ? "额外可写目录" : "Additional write roots"}
                value={fileValues("write", "roots").join(", ")}
                disabled={Boolean(props.disabled)}
                onInput={(event) => updateFileValues("write", "roots", event.currentTarget.value)}
              />
              <TextField
                label={props.zh ? "可写 Glob" : "Write globs"}
                value={fileValues("write", "globs").join(", ")}
                disabled={Boolean(props.disabled)}
                onInput={(event) => updateFileValues("write", "globs", event.currentTarget.value)}
              />
            </Show>
            <TextField
              label={props.zh ? "拒绝目录" : "Denied roots"}
              value={fileValues("deny", "roots").join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) => updateFileValues("deny", "roots", event.currentTarget.value)}
            />
            <TextField
              label={props.zh ? "拒绝 Glob" : "Denied globs"}
              value={fileValues("deny", "globs").join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) => updateFileValues("deny", "globs", event.currentTarget.value)}
            />
            <TextField
              label={props.zh ? "拒绝特殊根" : "Denied special roots"}
              value={fileValues("deny", "specialRoots").join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) =>
                updateFileValues("deny", "specialRoots", event.currentTarget.value)
              }
            />
          </section>

          <section>
            <h4>{props.zh ? "进程与网络" : "Process and network"}</h4>
            <Checkbox
              label={props.zh ? "允许沙箱进程" : "Allow sandboxed processes"}
              checked={props.value.rules.process.spawn}
              disabled={props.disabled || props.value.level === "read_only"}
              onChange={(event) =>
                updateRules({
                  process: { ...props.value.rules.process, spawn: event.currentTarget.checked },
                })
              }
            />
            <Checkbox
              label={props.zh ? "允许交互进程" : "Allow interactive processes"}
              checked={props.value.rules.process.interactive}
              disabled={props.disabled || props.value.level === "read_only"}
              onChange={(event) =>
                updateRules({
                  process: {
                    ...props.value.rules.process,
                    interactive: event.currentTarget.checked,
                  },
                })
              }
            />
            <TextField
              label={props.zh ? "允许的命令" : "Allowed commands"}
              value={props.value.rules.process.allowedCommands.join(", ")}
              disabled={props.disabled || props.value.level === "read_only"}
              onInput={(event) =>
                updateRules({
                  process: {
                    ...props.value.rules.process,
                    allowedCommands: splitValues(event.currentTarget.value),
                  },
                })
              }
            />
            <TextField
              label={props.zh ? "网络域名" : "Network hosts"}
              value={props.value.rules.network.hosts.join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) => {
                const hosts = splitValues(event.currentTarget.value);
                updateRules({
                  network: {
                    enabled: hosts.length > 0,
                    hosts,
                    protocols: hosts.length > 0 ? ["https"] : [],
                  },
                });
              }}
            />
          </section>

          <section>
            <h4>Browser</h4>
            <div data-component="permission-policy-checks">
              <For each={["observe", "act", "upload", "download", "cookieStorage", "cdp"] as const}>
                {(field) => (
                  <Checkbox
                    label={field}
                    checked={Boolean(props.value.rules.browser[field])}
                    disabled={
                      props.disabled || (props.value.level === "read_only" && field !== "observe")
                    }
                    onChange={(event) =>
                      updateRules({
                        browser: {
                          ...props.value.rules.browser,
                          [field]: event.currentTarget.checked,
                        },
                      })
                    }
                  />
                )}
              </For>
            </div>
            <TextField
              label={props.zh ? "允许的 Origin" : "Allowed origins"}
              value={(props.value.rules.browser.origins ?? []).join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) =>
                updateRules({
                  browser: {
                    ...props.value.rules.browser,
                    origins: splitValues(event.currentTarget.value),
                  },
                })
              }
            />
          </section>

          <section>
            <h4>{props.zh ? "桌面控制" : "Computer control"}</h4>
            <div data-component="permission-policy-checks">
              <Checkbox
                label={props.zh ? "观察" : "Observe"}
                checked={props.value.rules.computer.observe}
                disabled={Boolean(props.disabled)}
                onChange={(event) =>
                  updateRules({
                    computer: {
                      ...props.value.rules.computer,
                      observe: event.currentTarget.checked,
                    },
                  })
                }
              />
              <Checkbox
                label={props.zh ? "操作" : "Act"}
                checked={props.value.rules.computer.act}
                disabled={props.disabled || props.value.level === "read_only"}
                onChange={(event) =>
                  updateRules({
                    computer: { ...props.value.rules.computer, act: event.currentTarget.checked },
                  })
                }
              />
            </div>
            <TextField
              label={props.zh ? "应用窗口" : "Application windows"}
              value={props.value.rules.computer.targetWindows.join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) =>
                updateRules({
                  computer: {
                    ...props.value.rules.computer,
                    targetWindows: splitValues(event.currentTarget.value),
                  },
                })
              }
            />
            <TextField
              label={props.zh ? "最大操作数" : "Maximum actions"}
              type="number"
              value={props.value.rules.computer.maxActions?.toString() ?? ""}
              disabled={props.disabled || props.value.level === "read_only"}
              onInput={(event) => {
                const value = Number.parseInt(event.currentTarget.value, 10);
                updateRules({
                  computer: {
                    ...props.value.rules.computer,
                    maxActions: Number.isFinite(value) && value > 0 ? value : null,
                  },
                });
              }}
            />
          </section>
        </div>
      </Show>
    </div>
  );
}
