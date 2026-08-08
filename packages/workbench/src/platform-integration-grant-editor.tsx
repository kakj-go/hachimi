import {
  commands,
  type ChannelGrant,
  type ConnectorAccount,
  type ConnectorDriverDescriptor,
  type ConnectorRevisionSelection,
  type ContributionRevision,
  type McpServerView,
  type McpToolView,
  type SkillRecord,
} from "@hachimi/contracts";
import { Checkbox, SettingsCard, SettingsRow, TextField } from "@hachimi/ui";
import { For, Show, createSignal, onMount } from "solid-js";

import { PermissionPolicyEditor } from "./permission-policy-editor";

const splitValues = (value: string) => [
  ...new Set(
    value
      .split(/[\s,]+/)
      .map((item) => item.trim())
      .filter(Boolean),
  ),
];

export function ChannelGrantEditor(props: {
  value: ChannelGrant;
  disabled?: boolean;
  zh: boolean;
  onChange: (value: ChannelGrant) => void;
}) {
  const [skills, setSkills] = createSignal<SkillRecord[]>([]);
  const [mcpServers, setMcpServers] = createSignal<McpServerView[]>([]);
  const [mcpTools, setMcpTools] = createSignal<Record<string, McpToolView[]>>({});
  const [connectors, setConnectors] = createSignal<ConnectorOption[]>([]);
  const update = (patch: Partial<ChannelGrant>) => props.onChange({ ...props.value, ...patch });
  const hasSkill = (id: string) => props.value.skillIds.includes(id);
  const hasMcp = (id: string) => props.value.mcpServerIds.includes(id);
  const connector = (id: string) =>
    props.value.connectorSelections.find((value) => value.accountId === id);

  onMount(async () => {
    const optionalCommands = commands as typeof commands & {
      listSkills?: (projectId?: string) => Promise<SkillRecord[]>;
      listMcpServers?: () => Promise<McpServerView[]>;
      listMcpTools?: (serverId: string) => Promise<McpToolView[]>;
      listConnectorAccounts?: () => Promise<ConnectorAccount[]>;
      getConnectorDriverDescriptor?: (
        pluginId: string,
        connectorId: string,
      ) => Promise<ConnectorDriverDescriptor>;
      listPluginContributions?: (
        pluginId: string | null,
      ) => Promise<Array<{ contributionId: string; contentHash: string }>>;
    };
    try {
      const [nextSkills, nextMcp, nextAccounts] = await Promise.all([
        optionalCommands.listSkills?.() ?? Promise.resolve([]),
        optionalCommands.listMcpServers?.() ?? Promise.resolve([]),
        optionalCommands.listConnectorAccounts?.() ?? Promise.resolve([]),
      ]);
      setSkills(nextSkills.filter((skill) => skill.enabled));
      const readyMcp = nextMcp.filter((server) => server.health.state === "ready");
      setMcpServers(readyMcp);
      const toolsByServer = await Promise.all(
        readyMcp.map(
          async (server) =>
            [
              server.configuration.id,
              (await optionalCommands.listMcpTools?.(server.configuration.id)) ?? [],
            ] as const,
        ),
      );
      setMcpTools(Object.fromEntries(toolsByServer));
      const options = await Promise.all(
        nextAccounts.map((account) => connectorOption(optionalCommands, account)),
      );
      setConnectors(options.filter((value): value is ConnectorOption => value !== undefined));
    } catch {
      // The editor still exposes deterministic text fields when an optional catalog is unavailable.
    }
  });

  return (
    <div class="integration-grant-editor">
      <SettingsCard>
        <SettingsRow
          label={props.zh ? "Agent 权限" : "Agent permissions"}
          description={
            props.zh
              ? "此策略随 Channel binding 持久化，并在每次运行时生成不可变快照。"
              : "This policy persists with the Channel binding and is snapshotted for every Run."
          }
        >
          <PermissionPolicyEditor
            value={props.value.permissionPolicy}
            disabled={Boolean(props.disabled)}
            zh={props.zh}
            onChange={(permissionPolicy) =>
              update(
                permissionPolicy.level === "full_access"
                  ? {
                      permissionPolicy,
                      mcpServerIds: [],
                      connectorSelections: [],
                      readOnlyWorkspaceRoots: [],
                      networkHosts: [],
                    }
                  : { permissionPolicy },
              )
            }
          />
        </SettingsRow>
        <SettingsRow
          label={props.zh ? "Skills" : "Skills"}
          description={
            props.zh
              ? "仅允许明确选择的 Skill 进入 Channel Run。"
              : "Only explicitly selected Skills enter Channel Runs."
          }
        >
          <div class="integration-grant-options">
            <For each={skills()}>
              {(skill) => (
                <Checkbox
                  label={skill.qualifiedName}
                  checked={hasSkill(skill.id)}
                  disabled={props.disabled}
                  onChange={(event) =>
                    update({
                      skillIds: toggle(props.value.skillIds, skill.id, event.currentTarget.checked),
                    })
                  }
                />
              )}
            </For>
            <TextField
              label={props.zh ? "Skill ID（逗号分隔）" : "Skill IDs (comma separated)"}
              value={props.value.skillIds.join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) => update({ skillIds: splitValues(event.currentTarget.value) })}
            />
          </div>
        </SettingsRow>
        <Show when={props.value.permissionPolicy.level !== "full_access"}>
          <SettingsRow
            label={props.zh ? "MCP Servers" : "MCP Servers"}
            description={
              props.zh
                ? "运行前固定每个工具的 schema 与 Host revision。"
                : "Tool schema and Host revision are fixed before the Run."
            }
          >
            <div class="integration-grant-options">
              <For each={mcpServers()}>
                {(server) => (
                  <Checkbox
                    label={server.configuration.displayName}
                    checked={hasMcp(server.configuration.id)}
                    disabled={props.disabled}
                    onChange={(event) => {
                      const serverId = server.configuration.id;
                      const retained = props.value.permissionPolicy.rules.mcp.filter(
                        (rule) => rule.serverId !== serverId,
                      );
                      const additions = event.currentTarget.checked
                        ? (mcpTools()[serverId] ?? []).map((tool) => ({
                            serverId,
                            toolName: tool.name,
                            schemaHash: tool.schemaHash,
                            readOnly: server.configuration.readOnlyTools.includes(tool.name),
                          }))
                        : [];
                      update({
                        mcpServerIds: toggle(
                          props.value.mcpServerIds,
                          serverId,
                          event.currentTarget.checked,
                        ),
                        permissionPolicy: {
                          ...props.value.permissionPolicy,
                          rules: {
                            ...props.value.permissionPolicy.rules,
                            mcp: [...retained, ...additions],
                          },
                        },
                      });
                    }}
                  />
                )}
              </For>
              <TextField
                label={props.zh ? "MCP Server ID（逗号分隔）" : "MCP server IDs (comma separated)"}
                value={props.value.mcpServerIds.join(", ")}
                disabled={Boolean(props.disabled)}
                onInput={(event) =>
                  update({ mcpServerIds: splitValues(event.currentTarget.value) })
                }
              />
            </div>
          </SettingsRow>
        </Show>
        <Show when={props.value.permissionPolicy.level !== "full_access"}>
          <SettingsRow
            label={props.zh ? "Connectors" : "Connectors"}
            description={
              props.zh
                ? "每个账户都固定允许动作和 contribution revision。"
                : "Each account pins allowed actions and its contribution revision."
            }
          >
            <div class="integration-grant-options">
              <For each={connectors()}>
                {(option) => {
                  const selected = () => connector(option.account.id);
                  return (
                    <div class="integration-grant-connector">
                      <Checkbox
                        label={option.account.displayName}
                        checked={Boolean(selected())}
                        disabled={props.disabled || !option.revision}
                        onChange={(event) => {
                          const connectorSelections = toggleConnector(
                            props.value.connectorSelections,
                            option,
                            event.currentTarget.checked,
                          );
                          update({
                            connectorSelections,
                            permissionPolicy: withConnectorRules(
                              props.value.permissionPolicy,
                              connectorSelections,
                              connectors(),
                            ),
                          });
                        }}
                      />
                      <Show when={selected()}>
                        {(value) => (
                          <>
                            <div class="integration-grant-options">
                              <For each={option.descriptor.actions}>
                                {(action) => (
                                  <Checkbox
                                    label={`${action.name} · ${
                                      action.effect === "read_only"
                                        ? props.zh
                                          ? "只读"
                                          : "Read only"
                                        : props.zh
                                          ? "外部操作"
                                          : "External action"
                                    }`}
                                    checked={value().allowedActions.includes(action.name)}
                                    disabled={
                                      Boolean(props.disabled) ||
                                      (props.value.permissionPolicy.level === "read_only" &&
                                        action.effect !== "read_only")
                                    }
                                    onChange={(event) => {
                                      const connectorSelections =
                                        props.value.connectorSelections.map((selection) =>
                                          selection.accountId === option.account.id
                                            ? {
                                                ...selection,
                                                allowedActions: toggle(
                                                  selection.allowedActions,
                                                  action.name,
                                                  event.currentTarget.checked,
                                                ),
                                              }
                                            : selection,
                                        );
                                      update({
                                        connectorSelections,
                                        permissionPolicy: withConnectorRules(
                                          props.value.permissionPolicy,
                                          connectorSelections,
                                          connectors(),
                                        ),
                                      });
                                    }}
                                  />
                                )}
                              </For>
                            </div>
                          </>
                        )}
                      </Show>
                    </div>
                  );
                }}
              </For>
              <Show when={connectors().length === 0}>
                <span class="integration-route-empty">
                  {props.zh ? "没有可用 Connector 账户" : "No ready Connector accounts"}
                </span>
              </Show>
            </div>
          </SettingsRow>
        </Show>
        <Show when={props.value.permissionPolicy.level !== "full_access"}>
          <SettingsRow
            label={props.zh ? "只读 Workspace" : "Read-only Workspace"}
            description={
              props.zh ? "每行一个受控绝对路径。" : "One controlled absolute path per line."
            }
          >
            <TextField
              label={props.zh ? "只读路径" : "Read-only roots"}
              value={props.value.readOnlyWorkspaceRoots.join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) =>
                update({ readOnlyWorkspaceRoots: splitValues(event.currentTarget.value) })
              }
            />
          </SettingsRow>
        </Show>
        <Show when={props.value.permissionPolicy.level !== "full_access"}>
          <SettingsRow
            label={props.zh ? "网络范围" : "Network scope"}
            description={
              props.zh
                ? "只允许显式列出的 HTTPS 主机。"
                : "Only explicitly listed HTTPS hosts are allowed."
            }
          >
            <TextField
              label={props.zh ? "Network hosts" : "Network hosts"}
              value={props.value.networkHosts.join(", ")}
              disabled={Boolean(props.disabled)}
              onInput={(event) => update({ networkHosts: splitValues(event.currentTarget.value) })}
            />
          </SettingsRow>
        </Show>
      </SettingsCard>
    </div>
  );
}

interface ConnectorOption {
  account: ConnectorAccount;
  descriptor: ConnectorDriverDescriptor;
  revision: ContributionRevision | null;
}

async function connectorOption(
  available: {
    getConnectorDriverDescriptor?: (
      pluginId: string,
      connectorId: string,
    ) => Promise<ConnectorDriverDescriptor>;
    listPluginContributions?: (
      pluginId: string | null,
    ) => Promise<Array<{ contributionId: string; contentHash: string }>>;
  },
  account: ConnectorAccount,
): Promise<ConnectorOption | undefined> {
  if (!available.getConnectorDriverDescriptor) return undefined;
  const descriptor = await available.getConnectorDriverDescriptor(
    account.pluginId,
    account.connectorId,
  );
  const contributions = available.listPluginContributions
    ? await available.listPluginContributions(account.pluginId)
    : [];
  const contribution = contributions.find((value) => value.contributionId === account.connectorId);
  const revision = contribution
    ? {
        pluginId: account.pluginId,
        contributionId: account.connectorId,
        accountId: account.id,
        contentHash: contribution.contentHash,
        hostIdentityHash: account.revision.hostIdentityHash,
        schemaHash: account.revision.schemaHash,
        actionHash: account.revision.actionHash,
      }
    : null;
  return { account, descriptor, revision };
}

function toggle(values: string[], value: string, checked: boolean): string[] {
  return checked ? [...new Set([...values, value])] : values.filter((item) => item !== value);
}

function toggleConnector(
  values: ConnectorRevisionSelection[],
  option: ConnectorOption,
  checked: boolean,
): ConnectorRevisionSelection[] {
  if (!checked) return values.filter((value) => value.accountId !== option.account.id);
  if (!option.revision) return values;
  return [
    ...values.filter((value) => value.accountId !== option.account.id),
    {
      accountId: option.account.id,
      contributionRevision: option.revision,
      allowedActions: option.descriptor.actions.map((action) => action.name),
    },
  ];
}

function withConnectorRules(
  policy: ChannelGrant["permissionPolicy"],
  selections: ConnectorRevisionSelection[],
  options: ConnectorOption[],
): ChannelGrant["permissionPolicy"] {
  return {
    ...policy,
    rules: {
      ...policy.rules,
      connectors: selections.map((selection) => {
        const descriptor = options.find(
          (option) => option.account.id === selection.accountId,
        )?.descriptor;
        const readOnlyActions = selection.allowedActions.filter((name) =>
          descriptor?.actions.some(
            (action) => action.name === name && action.effect === "read_only",
          ),
        );
        return {
          accountId: selection.accountId,
          actions: [...selection.allowedActions],
          readOnlyActions,
          contributionRevision: selection.contributionRevision.actionHash ?? "",
        };
      }),
    },
  };
}
