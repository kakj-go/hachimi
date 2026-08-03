import type {
  McpServerView,
  McpToolView,
  SkillFileSnapshot,
  SkillRecord,
  SkillTreeNode,
} from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { For, Show, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { McpSettingsPage } from "./mcp-settings";
import { SkillsSettingsPage } from "./skills-settings";

const commandMocks = vi.hoisted(() => ({
  listSkills: vi.fn(),
  createSkill: vi.fn(),
  importSkillArchive: vi.fn(),
  importSkillDroppedFiles: vi.fn(),
  renameSkill: vi.fn(),
  removeSkill: vi.fn(),
  setSkillEnabled: vi.fn(),
  getSkillTree: vi.fn(),
  readSkillFile: vi.fn(),
  readSkillPreviewResource: vi.fn(),
  writeSkillFile: vi.fn(),
  createSkillEntry: vi.fn(),
  renameSkillEntry: vi.fn(),
  removeSkillEntry: vi.fn(),
  validateSkill: vi.fn(),
  subscribeSkills: vi.fn(),
  unsubscribeSkills: vi.fn(),
  listMcpServers: vi.fn(),
  getMcpEchoServerUrl: vi.fn(),
  getMcpServer: vi.fn(),
  testMcpServer: vi.fn(),
  upsertMcpServer: vi.fn(),
  setMcpServerEnabled: vi.fn(),
  refreshMcpServer: vi.fn(),
  removeMcpServer: vi.fn(),
  listMcpTools: vi.fn(),
  discoverMcpTools: vi.fn(),
  setMcpToolEnabled: vi.fn(),
  getMcpInventory: vi.fn(),
  listMcpCallSummaries: vi.fn(),
  refreshMcpInventory: vi.fn(),
  readMcpResource: vi.fn(),
  getMcpPrompt: vi.fn(),
  getMcpAuthStatus: vi.fn(),
  startMcpOAuthLogin: vi.fn(),
  logoutMcpOAuth: vi.fn(),
}));

const eventState = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@hachimi/contracts", () => ({
  commands: commandMocks,
  commandFailure: (reason: unknown) => ({ code: "command_failed", message: String(reason) }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    eventState.handlers.set(name, handler);
    return () => {
      eventState.handlers.delete(name);
    };
  }),
}));

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  const Button = (props: {
    children?: JSX.Element;
    disabled?: boolean;
    onClick?: () => void;
    "data-testid"?: string;
  }) => (
    <button
      type="button"
      data-testid={props["data-testid"]}
      disabled={props.disabled}
      onClick={() => props.onClick?.()}
    >
      {props.children}
    </button>
  );
  const TextField = (props: {
    label: string;
    value?: string;
    placeholder?: string;
    type?: string;
    onInput?: JSX.EventHandler<HTMLInputElement, InputEvent>;
  }) => (
    <label>
      {props.label}
      <input
        value={props.value ?? ""}
        placeholder={props.placeholder}
        type={props.type ?? "text"}
        onInput={(event) => props.onInput?.(event)}
      />
    </label>
  );
  return {
    Button,
    Workspace: (props: { children?: JSX.Element }) => <section>{props.children}</section>,
    Dialog: (props: { open: boolean; children?: JSX.Element; title: string }) => (
      <Show when={props.open}>
        <div role="dialog" aria-label={props.title}>
          {props.children}
        </div>
      </Show>
    ),
    Dropdown: (props: {
      label: string;
      triggerTestId?: string;
      children?: JSX.Element;
      actions: readonly {
        id: string;
        label: string;
        testId?: string;
        disabled?: boolean;
      }[];
      onSelect: (id: string) => void;
    }) => (
      <div>
        <button type="button" data-testid={props.triggerTestId} aria-label={props.label}>
          {props.children}
        </button>
        <For each={props.actions}>
          {(action) => (
            <button
              type="button"
              data-testid={action.testId}
              disabled={action.disabled}
              onClick={() => props.onSelect(action.id)}
            >
              {action.label}
            </button>
          )}
        </For>
      </div>
    ),
    Folder: Icon,
    FolderOpen: Icon,
    FileText: Icon,
    Code2: Icon,
    MoreHorizontal: Icon,
    PageHeading: (props: {
      title: string;
      description?: string;
      actions?: JSX.Element;
      class?: string;
    }) => (
      <header class={props.class}>
        <h1>{props.title}</h1>
        <p>{props.description}</p>
        <div>{props.actions}</div>
      </header>
    ),
    Plus: Icon,
    Upload: Icon,
    RefreshCw: Icon,
    SegmentedControl: (props: {
      label: string;
      value: string;
      options: readonly { value: string; label: string }[];
      onChange: (value: string) => void;
    }) => (
      <div aria-label={props.label}>
        <For each={props.options}>
          {(option) => (
            <button type="button" onClick={() => props.onChange(option.value)}>
              {option.label}
            </button>
          )}
        </For>
      </div>
    ),
    StatusBanner: (props: { children?: JSX.Element }) => <div role="status">{props.children}</div>,
    Switch: (props: {
      checked: boolean;
      disabled?: boolean;
      label: string;
      onChange?: (checked: boolean) => void;
    }) => (
      <button
        type="button"
        role="switch"
        aria-label={props.label}
        aria-checked={props.checked}
        disabled={props.disabled}
        onClick={() => props.onChange?.(!props.checked)}
      />
    ),
    TextArea: (props: {
      label: string;
      value?: string;
      onInput?: JSX.EventHandler<HTMLTextAreaElement, InputEvent>;
    }) => (
      <textarea
        aria-label={props.label}
        value={props.value ?? ""}
        onInput={(event) => props.onInput?.(event)}
      />
    ),
    TextField,
    Trash2: Icon,
  };
});

const skill: SkillRecord = {
  id: "skill-1",
  scope: "user",
  namespace: null,
  name: "release-notes",
  qualifiedName: "release-notes",
  description: "Prepare releases",
  dependencies: [],
  editable: true,
  enabled: true,
  contentHash: "entry-hash",
  treeRevision: "tree-1",
  diagnostics: [],
  updatedAtMs: 1,
};

const tree: SkillTreeNode = {
  name: "release-notes",
  relativePath: "",
  kind: "directory",
  editorKind: "unsupported",
  sizeBytes: 0,
  revision: null,
  children: [
    {
      name: "SKILL.md",
      relativePath: "SKILL.md",
      kind: "file",
      editorKind: "markdown",
      sizeBytes: 10,
      revision: "entry-1",
      children: [],
    },
  ],
};

const snapshot: SkillFileSnapshot = {
  skillId: skill.id,
  relativePath: "SKILL.md",
  editorKind: "markdown",
  content: "---\nname: release-notes\ndescription: Prepare releases\n---\n",
  sizeBytes: 63,
  revision: "entry-1",
  diagnostics: [],
};

const mcpServer: McpServerView = {
  configuration: {
    id: "mcp-1",
    displayName: "Docs",
    enabled: true,
    transport: { kind: "streamable_http", url: "https://example.com/mcp" },
    headers: [],
    readOnlyTools: [],
    startupTimeoutMs: 15_000,
    requestTimeoutMs: 60_000,
    maxMessageBytes: 1_048_576,
    createdAtMs: 1,
    updatedAtMs: 1,
  },
  health: {
    serverId: "mcp-1",
    state: "ready",
    serverName: "Docs",
    serverVersion: "1",
    protocolVersion: "2025-06-18",
    toolCount: 1,
    errorCode: null,
    checkedAtMs: 1,
  },
};

const mcpTool: McpToolView = {
  serverId: "mcp-1",
  name: "search",
  exposedName: "mcp__mcp_1__search",
  description: "Search docs",
  inputSchema: {
    type: "object",
    properties: { query: { type: "string", description: "Search query" } },
    required: ["query"],
  },
  requiredParameters: ["query"],
  enabled: true,
  stale: false,
  validationError: null,
  schemaHash: "schema-1",
  hostIdentityHash: "host-v1",
  discoveredAtMs: 1,
};

async function settle() {
  for (let index = 0; index < 8; index += 1) await Promise.resolve();
}

function mount(children: () => JSX.Element) {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(
    () => <I18nProvider initialLocale="en-US">{children()}</I18nProvider>,
    host,
  );
  return { host, dispose };
}

beforeEach(() => {
  vi.clearAllMocks();
  eventState.handlers.clear();
  commandMocks.listSkills.mockResolvedValue([skill]);
  commandMocks.getSkillTree.mockResolvedValue(tree);
  commandMocks.readSkillFile.mockResolvedValue(snapshot);
  commandMocks.writeSkillFile.mockResolvedValue({ ...snapshot, revision: "entry-2" });
  commandMocks.importSkillArchive.mockResolvedValue(null);
  commandMocks.importSkillDroppedFiles.mockResolvedValue(tree);
  commandMocks.setSkillEnabled.mockResolvedValue({ ...skill, enabled: false });
  commandMocks.subscribeSkills.mockResolvedValue("subscription-1");
  commandMocks.unsubscribeSkills.mockResolvedValue(true);
  commandMocks.listMcpServers.mockResolvedValue([mcpServer]);
  commandMocks.getMcpEchoServerUrl.mockResolvedValue("http://127.0.0.1:43123/mcp");
  commandMocks.getMcpServer.mockResolvedValue(mcpServer);
  commandMocks.listMcpTools.mockResolvedValue([mcpTool]);
  commandMocks.discoverMcpTools.mockResolvedValue({
    success: true,
    serverName: "Docs",
    serverVersion: "1",
    protocolVersion: "2025-06-18",
    tools: [mcpTool],
    errorCode: null,
  });
  commandMocks.setMcpToolEnabled.mockResolvedValue({ ...mcpTool, enabled: false });
  const inventory = {
    serverId: "mcp-1",
    resources: [
      {
        uri: "docs://guide",
        name: "guide",
        title: "Guide",
        description: "Product guide",
        mimeType: "text/plain",
        size: 12,
        annotations: null,
        meta: null,
      },
    ],
    resourceTemplates: [
      {
        uriTemplate: "docs://{page}",
        name: "page",
        title: null,
        description: null,
        mimeType: "text/plain",
        annotations: null,
        meta: null,
      },
    ],
    prompts: [
      {
        name: "brief",
        title: null,
        description: "Prepare a brief",
        arguments: [],
      },
    ],
    errors: {},
    stale: false,
    refreshedAtMs: 2,
  };
  commandMocks.getMcpInventory.mockResolvedValue(inventory);
  commandMocks.listMcpCallSummaries.mockResolvedValue([
    {
      id: "call-1",
      serverId: "mcp-1",
      sessionId: "session-1",
      runId: "run-1",
      toolName: "search",
      outcome: "succeeded",
      durationMs: 24,
      createdAtMs: 1_700_000_000_000,
    },
  ]);
  commandMocks.refreshMcpInventory.mockResolvedValue(inventory);
  commandMocks.readMcpResource.mockResolvedValue([
    { uri: "docs://guide", mimeType: "text/plain", text: "Guide text", blobBase64: null },
  ]);
  commandMocks.getMcpPrompt.mockResolvedValue({
    description: "Brief",
    messages: [{ role: "user", content: { type: "text", text: "Prepare it" } }],
  });
  commandMocks.getMcpAuthStatus.mockResolvedValue({
    serverId: "mcp-1",
    status: "not_logged_in",
    scopesSupported: ["mcp.read"],
  });
  commandMocks.startMcpOAuthLogin.mockResolvedValue({
    authorizationUrl: "https://identity.example.test/authorize?state=redacted",
  });
  commandMocks.logoutMcpOAuth.mockResolvedValue({
    serverId: "mcp-1",
    status: "not_logged_in",
    scopesSupported: ["mcp.read"],
  });
});

afterEach(() => {
  document.body.replaceChildren();
  Reflect.deleteProperty(document, "elementFromPoint");
});

describe("extension settings pages", () => {
  it("loads and unsubscribes the real Skill projection", async () => {
    const mounted = mount(() => <SkillsSettingsPage />);
    await settle();
    expect(commandMocks.subscribeSkills).toHaveBeenCalledTimes(1);
    expect(mounted.host.querySelector('[data-testid="skill-node-SKILL.md"]')).not.toBeNull();
    expect(mounted.host.querySelector('[data-testid="skill-markdown-editor"]')).not.toBeNull();
    expect(mounted.host.querySelector('[data-testid="skill-editor-input"]')).toBeNull();
    expect(mounted.host.textContent).toContain("release-notes");
    mounted.dispose();
    await settle();
    expect(commandMocks.unsubscribeSkills).toHaveBeenCalledWith("subscription-1");
  });

  it("removes the old editor while a newly created Skill file is still loading", async () => {
    const mounted = mount(() => <SkillsSettingsPage />);
    await settle();
    const nextTree: SkillTreeNode = {
      ...tree,
      children: [
        ...tree.children,
        {
          name: "reference.md",
          relativePath: "reference.md",
          kind: "file",
          editorKind: "markdown",
          sizeBytes: 0,
          revision: "reference-1",
          children: [],
        },
      ],
    };
    commandMocks.createSkillEntry.mockResolvedValue(nextTree);
    let resolveFile!: (value: SkillFileSnapshot) => void;
    commandMocks.readSkillFile.mockImplementationOnce(
      () =>
        new Promise<SkillFileSnapshot>((resolve) => {
          resolveFile = resolve;
        }),
    );
    mounted.host
      .querySelector<HTMLElement>('[data-testid="skill-action-new-file-release-notes"]')
      ?.click();
    await settle();
    const dialog = mounted.host.querySelector<HTMLElement>('[role="dialog"]');
    const input = dialog?.querySelector<HTMLInputElement>("input");
    if (input) {
      input.value = "reference.md";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    }
    [...(dialog?.querySelectorAll("button") ?? [])]
      .find((button) => button.textContent === "Create")
      ?.click();
    await settle();
    expect(mounted.host.querySelector('[data-testid="skill-markdown-editor"]')).toBeNull();

    resolveFile({
      ...snapshot,
      relativePath: "reference.md",
      content: "",
      sizeBytes: 0,
      revision: "reference-1",
    });
    await settle();
    expect(mounted.host.querySelector('[data-testid="skill-markdown-editor"]')).not.toBeNull();
    mounted.dispose();
  });

  it("exposes tree actions behind an ellipsis and uses the shared rename dialog", async () => {
    const mounted = mount(() => <SkillsSettingsPage />);
    await settle();
    expect(mounted.host.querySelector('[aria-label="Actions for release-notes"]')).not.toBeNull();
    expect(mounted.host.querySelector('[role="switch"]')).toBeNull();
    expect(mounted.host.querySelector('[aria-label="Resize Skill list"]')).not.toBeNull();
    mounted.host
      .querySelector<HTMLElement>('[data-testid="skill-action-toggle-release-notes"]')
      ?.click();
    await settle();
    expect(commandMocks.setSkillEnabled).toHaveBeenCalledWith("skill-1", false);
    const rename = [...mounted.host.querySelectorAll("button")].find(
      (button) => button.textContent === "Rename",
    );
    rename?.click();
    await settle();
    const dialog = mounted.host.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog?.getAttribute("aria-label")).toBe("Rename Skill");
    const input = dialog?.querySelector<HTMLInputElement>("input");
    if (input) {
      input.value = "release-helper";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    }
    const submit = [...(dialog?.querySelectorAll("button") ?? [])].find(
      (button) => button.textContent === "Rename",
    );
    submit?.click();
    await settle();
    expect(commandMocks.renameSkill).toHaveBeenCalledWith("skill-1", "release-helper");
    mounted.dispose();
  });

  it("imports a validated Skill ZIP through the Desktop command", async () => {
    commandMocks.importSkillArchive.mockResolvedValue(skill);
    const mounted = mount(() => <SkillsSettingsPage />);
    await settle();
    mounted.host.querySelector<HTMLElement>('[data-testid="skill-import"]')?.click();
    await settle();
    expect(commandMocks.importSkillArchive).toHaveBeenCalledTimes(1);
    expect(commandMocks.listSkills).toHaveBeenCalledTimes(2);
    mounted.dispose();
  });

  it("imports native Windows file drops into the hovered Skill directory", async () => {
    const mounted = mount(() => <SkillsSettingsPage />);
    await settle();
    const root = mounted.host.querySelector<HTMLElement>("[data-skill-drop-skill-id]");
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => root),
    });
    eventState.handlers.get("skills:native-drag")?.({
      payload: {
        kind: "drop",
        token: "drop-token",
        x: 40,
        y: 80,
        fileNames: ["guide.md", "helper.py"],
      },
    });
    await settle();
    expect(commandMocks.importSkillDroppedFiles).toHaveBeenCalledWith("drop-token", "skill-1", "");
    expect(mounted.host.textContent).toContain("Imported 2 file(s)");
    mounted.dispose();
  });

  it("shows MCP schema details and persists a Tool disable", async () => {
    const mounted = mount(() => <McpSettingsPage connectorEnabled />);
    await settle();
    expect(commandMocks.getMcpServer).toHaveBeenCalledWith("mcp-1");
    expect(commandMocks.discoverMcpTools).toHaveBeenCalledWith("mcp-1");
    expect(commandMocks.refreshMcpInventory).toHaveBeenCalledWith("mcp-1");
    expect(mounted.host.textContent).toContain("Search query");
    expect(mounted.host.textContent).toContain("Product guide");
    expect(mounted.host.textContent).toContain("Prepare a brief");
    expect(mounted.host.textContent).toContain("Recent calls");
    expect(mounted.host.textContent).toContain("24 ms");
    expect(mounted.host.querySelector(".mcp-advanced-transport")).not.toBeNull();
    mounted.host.querySelector<HTMLElement>('[aria-label="Expose search to the Agent"]')?.click();
    await settle();
    expect(commandMocks.setMcpToolEnabled).toHaveBeenCalledWith("mcp-1", "search", false);
    expect(mounted.host.textContent).toContain("Expose to Agent");
    mounted.dispose();
  });

  it("starts OAuth in a browser and sends only server ID and public scopes", async () => {
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    const mounted = mount(() => <McpSettingsPage connectorEnabled />);
    await settle();
    mounted.host.querySelector<HTMLElement>('[data-testid="mcp-oauth-login"]')?.click();
    await settle();
    expect(commandMocks.startMcpOAuthLogin).toHaveBeenCalledWith({
      serverId: "mcp-1",
      scopes: ["mcp.read"],
      timeoutSecs: 300,
    });
    expect(open).toHaveBeenCalledWith(
      "https://identity.example.test/authorize?state=redacted",
      "_blank",
      "noopener,noreferrer",
    );
    expect(mounted.host.textContent).not.toContain("access_token");
    mounted.dispose();
    open.mockRestore();
  });

  it("logs out an OAuth server through the keyring-backed command", async () => {
    commandMocks.getMcpAuthStatus.mockResolvedValue({
      serverId: "mcp-1",
      status: "oauth",
      scopesSupported: ["mcp.read"],
    });
    const mounted = mount(() => <McpSettingsPage connectorEnabled />);
    await settle();
    mounted.host.querySelector<HTMLElement>('[data-testid="mcp-oauth-logout"]')?.click();
    await settle();
    expect(commandMocks.logoutMcpOAuth).toHaveBeenCalledWith("mcp-1");
    expect(mounted.host.textContent).toContain("This server supports OAuth");
    mounted.dispose();
  });

  it("opens MCP creation in a dialog and offers the running Echo endpoint", async () => {
    const mounted = mount(() => <McpSettingsPage connectorEnabled />);
    await settle();
    mounted.host.querySelector<HTMLElement>('[data-testid="mcp-add-server"]')?.click();
    await settle();
    const dialog = mounted.host.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog?.getAttribute("aria-label")).toBe("Add MCP server");
    expect(dialog?.textContent).toContain("Built-in Echo test server");
    dialog?.querySelector<HTMLElement>('[data-testid="mcp-use-echo-server"]')?.click();
    await settle();
    const inputs = dialog?.querySelectorAll<HTMLInputElement>("input");
    expect(inputs?.[0]?.value).toBe("Hachimi Echo");
    expect(inputs?.[1]?.value).toBe("http://127.0.0.1:43123/mcp");
    expect(dialog?.querySelector('[data-testid="mcp-test-new-connection"]')).not.toBeNull();
    mounted.dispose();
  });

  it("tests and saves a new MCP server through the create dialog", async () => {
    commandMocks.listMcpServers.mockResolvedValueOnce([]).mockResolvedValue([mcpServer]);
    commandMocks.testMcpServer.mockResolvedValue({
      success: true,
      serverName: "Echo",
      serverVersion: "1",
      protocolVersion: "2025-06-18",
      tools: [{ ...mcpTool, name: "echo", exposedName: "mcp__draft__echo" }],
      errorCode: null,
    });
    commandMocks.upsertMcpServer.mockResolvedValue(mcpServer);
    const mounted = mount(() => <McpSettingsPage connectorEnabled />);
    await settle();
    mounted.host.querySelector<HTMLElement>('[data-testid="mcp-add-server"]')?.click();
    await settle();
    const dialog = mounted.host.querySelector<HTMLElement>('[role="dialog"]');
    const name = dialog?.querySelector<HTMLInputElement>('[placeholder*="Filesystem"]');
    const url = dialog?.querySelector<HTMLInputElement>('[placeholder="https://example.com/mcp"]');
    if (name) {
      name.value = "Echo service";
      name.dispatchEvent(new InputEvent("input", { bubbles: true }));
    }
    if (url) {
      url.value = "https://localhost.invalid/mcp";
      url.dispatchEvent(new InputEvent("input", { bubbles: true }));
    }
    dialog?.querySelector<HTMLElement>('[data-testid="mcp-test-new-connection"]')?.click();
    await settle();
    expect(commandMocks.testMcpServer).toHaveBeenCalledTimes(1);
    expect(dialog?.querySelector('[data-testid="mcp-tool-echo"]')).not.toBeNull();
    dialog?.querySelector<HTMLElement>('[data-testid="mcp-save-new-server"]')?.click();
    await settle();
    expect(commandMocks.upsertMcpServer).toHaveBeenCalledWith(
      expect.objectContaining({ displayName: "Echo service", enabled: false }),
    );
    mounted.dispose();
  });

  it("keeps MCP configuration visible when the runtime feature is disabled", async () => {
    const mounted = mount(() => <McpSettingsPage connectorEnabled={false} />);
    await settle();
    expect(mounted.host.textContent).toContain("Connector Runtime is disabled");
    expect(mounted.host.querySelector('[data-testid="mcp-save-server"]')).not.toBeNull();
    mounted.dispose();
  });
});
