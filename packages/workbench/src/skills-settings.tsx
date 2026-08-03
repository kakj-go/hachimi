import {
  commandFailure,
  commands,
  type SkillEntryKind,
  type SkillChangeEvent,
  type SkillFileSnapshot,
  type SkillPreviewResource,
  type SkillRecord,
  type SkillSubscriptionId,
  type SkillTreeNode,
} from "@hachimi/contracts";
import { listen } from "@tauri-apps/api/event";
import { useI18n } from "@hachimi/i18n";
import {
  Button,
  Code2,
  Dialog,
  Dropdown,
  type DropdownAction,
  FileText,
  Folder,
  FolderOpen,
  MoreHorizontal,
  PageHeading,
  Plus,
  StatusBanner,
  TextField,
  Upload,
  Workspace,
} from "@hachimi/ui";
import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";

import { TextEditor } from "./text-editor";

type RenameTarget =
  | { kind: "skill"; skillId: string; currentName: string }
  | {
      kind: "entry";
      skillId: string;
      relativePath: string;
      currentName: string;
    };

interface ConfirmationRequest {
  title: string;
  description: string;
  confirmLabel: string;
  danger?: boolean;
  run: () => Promise<void>;
}

interface SkillNativeDragEvent {
  kind: "enter" | "over" | "drop" | "leave";
  token: string | null;
  x: number | null;
  y: number | null;
  fileNames: string[];
}

interface SkillDropTarget {
  skillId: string;
  parentPath: string;
  key: string;
}

export function SkillsSettingsPage() {
  const i18n = useI18n();
  const copy = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const [skills, setSkills] = createSignal<SkillRecord[]>([]);
  const [selectedSkillId, setSelectedSkillId] = createSignal<string>();
  const [tree, setTree] = createSignal<SkillTreeNode>();
  const [selectedPath, setSelectedPath] = createSignal("SKILL.md");
  const [file, setFile] = createSignal<SkillFileSnapshot>();
  const [draft, setDraft] = createSignal("");
  const [dirty, setDirty] = createSignal(false);
  const [conflict, setConflict] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [loading, setLoading] = createSignal(true);
  const [importing, setImporting] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const [createOpen, setCreateOpen] = createSignal(false);
  const [newSkillName, setNewSkillName] = createSignal("");
  const [entryOpen, setEntryOpen] = createSignal(false);
  const [entryName, setEntryName] = createSignal("");
  const [entryKind, setEntryKind] = createSignal<SkillEntryKind>("file");
  const [entrySkillId, setEntrySkillId] = createSignal<string>();
  const [entryParentPath, setEntryParentPath] = createSignal("");
  const [renameTarget, setRenameTarget] = createSignal<RenameTarget>();
  const [renameName, setRenameName] = createSignal("");
  const [confirmation, setConfirmation] = createSignal<ConfirmationRequest>();
  const [confirming, setConfirming] = createSignal(false);
  const [skillListWidth, setSkillListWidth] = createSignal(290);
  const [dropActive, setDropActive] = createSignal(false);
  const [dropTargetKey, setDropTargetKey] = createSignal<string>();
  const [dropNotice, setDropNotice] = createSignal<string>();
  let workspaceElement: HTMLElement | undefined;
  let stopChanges: (() => void) | undefined;
  let stopNativeDrag: (() => void) | undefined;
  let skillSubscriptionId: SkillSubscriptionId | undefined;
  let skillSelectionGeneration = 0;
  let fileLoadGeneration = 0;
  let externalRefreshGeneration = 0;

  const selectedSkill = createMemo(() => skills().find((skill) => skill.id === selectedSkillId()));

  async function loadSkills(selectId?: string) {
    const next = await commands.listSkills();
    setSkills(next);
    const id = selectId ?? selectedSkillId() ?? next[0]?.id;
    if (id && id !== selectedSkillId()) await selectSkill(id, true);
  }

  async function selectSkill(skillId: string, force = false) {
    if (!force && dirty()) {
      setConfirmation({
        title: copy("放弃未保存的修改？", "Discard unsaved changes?"),
        description: copy(
          "切换 Skill 会丢弃当前文件尚未保存的修改。",
          "Switching Skills will discard the unsaved changes in the current file.",
        ),
        confirmLabel: copy("放弃并切换", "Discard and switch"),
        danger: true,
        run: () => selectSkill(skillId, true),
      });
      return;
    }
    const generation = ++skillSelectionGeneration;
    fileLoadGeneration += 1;
    externalRefreshGeneration += 1;
    setSelectedSkillId(skillId);
    setSelectedPath("");
    setFile();
    setDraft("");
    setDirty(false);
    setConflict(false);
    const nextTree = await commands.getSkillTree(skillId);
    if (generation !== skillSelectionGeneration || selectedSkillId() !== skillId) return;
    setTree(nextTree);
    if (findNode(nextTree, "SKILL.md")) {
      setSelectedPath("SKILL.md");
      await loadFile(skillId, "SKILL.md", true);
    } else {
      fileLoadGeneration += 1;
    }
  }

  async function loadFile(skillId: string, relativePath: string, force = false) {
    if (!force && dirty()) {
      setConfirmation({
        title: copy("放弃未保存的修改？", "Discard unsaved changes?"),
        description: copy(
          "切换文件会丢弃当前文件尚未保存的修改。",
          "Switching files will discard the unsaved changes in the current file.",
        ),
        confirmLabel: copy("放弃并切换", "Discard and switch"),
        danger: true,
        run: () => loadFile(skillId, relativePath, true),
      });
      return;
    }
    const generation = ++fileLoadGeneration;
    externalRefreshGeneration += 1;
    setSelectedPath(relativePath);
    setFile();
    setDraft("");
    setDirty(false);
    setConflict(false);
    const snapshot = await commands.readSkillFile(skillId, relativePath);
    if (
      generation !== fileLoadGeneration ||
      selectedSkillId() !== skillId ||
      selectedPath() !== relativePath
    )
      return;
    setFile(snapshot);
    setDraft(snapshot.content ?? "");
    setDirty(false);
    setConflict(false);
  }

  async function refreshExternalChanges() {
    const refreshGeneration = ++externalRefreshGeneration;
    try {
      const nextSkills = await commands.listSkills();
      if (refreshGeneration !== externalRefreshGeneration) return;
      setSkills(nextSkills);
      const skillId = selectedSkillId();
      if (skillId) {
        const nextTree = await commands.getSkillTree(skillId);
        if (refreshGeneration !== externalRefreshGeneration || selectedSkillId() !== skillId)
          return;
        setTree(nextTree);
      }
      const current = file();
      const currentFileLoadGeneration = fileLoadGeneration;
      if (skillId && current) {
        const disk = await commands.readSkillFile(skillId, current.relativePath);
        const active = file();
        if (
          refreshGeneration !== externalRefreshGeneration ||
          currentFileLoadGeneration !== fileLoadGeneration ||
          selectedSkillId() !== skillId ||
          !active ||
          active.skillId !== current.skillId ||
          active.relativePath !== current.relativePath ||
          active.revision !== current.revision
        )
          return;
        if (disk.revision !== current.revision) {
          if (dirty()) setConflict(true);
          else {
            setFile(disk);
            setDraft(disk.content ?? "");
          }
        }
      }
    } catch {
      // Watcher errors are surfaced by the next explicit operation.
    }
  }

  async function createSkill() {
    try {
      const created = await commands.createSkill(newSkillName().trim());
      setCreateOpen(false);
      setNewSkillName("");
      await loadSkills(created.id);
    } catch (reason) {
      setError(commandFailure(reason).message);
    }
  }

  async function importSkillArchive() {
    setImporting(true);
    setError();
    try {
      const imported = await commands.importSkillArchive();
      if (!imported) return;
      setSkills(await commands.listSkills());
      if (!dirty()) await selectSkill(imported.id, true);
    } catch (reason) {
      setError(commandFailure(reason).message);
    } finally {
      setImporting(false);
    }
  }

  async function setSkillEnabled(skillId: string, enabled: boolean) {
    try {
      const updated = await commands.setSkillEnabled(skillId, enabled);
      setSkills((current) => current.map((skill) => (skill.id === updated.id ? updated : skill)));
    } catch (reason) {
      setError(commandFailure(reason).message);
    }
  }

  async function save() {
    const snapshot = file();
    if (!snapshot || snapshot.content === null) return;
    setSaving(true);
    setError();
    try {
      const saved = await commands.writeSkillFile({
        skillId: snapshot.skillId,
        relativePath: snapshot.relativePath,
        content: draft(),
        expectedRevision: snapshot.revision,
      });
      setFile(saved);
      setDraft(saved.content ?? "");
      setDirty(false);
      setConflict(false);
      setTree(await commands.getSkillTree(snapshot.skillId));
      setSkills(await commands.listSkills());
    } catch (reason) {
      const failure = commandFailure(reason);
      if (failure.code === "skill_write_failed" && failure.message.includes("changed")) {
        setConflict(true);
      }
      setError(failure.message);
    } finally {
      setSaving(false);
    }
  }

  async function keepLocalDraft() {
    const current = file();
    if (!current) return;
    try {
      const disk = await commands.readSkillFile(current.skillId, current.relativePath);
      setFile(disk);
      setConflict(false);
      setDirty(true);
    } catch (reason) {
      setError(commandFailure(reason).message);
    }
  }

  function resolvePreviewReference(destination: string): Promise<SkillPreviewResource> {
    const current = file();
    if (!current) return Promise.reject(new Error("No Skill file is selected"));
    return commands.readSkillPreviewResource({
      skillId: current.skillId,
      sourcePath: current.relativePath,
      destination,
    });
  }

  function openEntryDialog(skillId: string, parentPath: string, kind: SkillEntryKind): void {
    setEntrySkillId(skillId);
    setEntryParentPath(parentPath);
    setEntryKind(kind);
    setEntryName("");
    setEntryOpen(true);
  }

  async function createEntry() {
    const skillId = entrySkillId();
    if (!skillId) return;
    try {
      const parentPath = entryParentPath();
      const name = entryName().trim();
      const updatedTree = await commands.createSkillEntry({
        skillId,
        parentPath,
        name,
        kind: entryKind(),
      });
      setEntryOpen(false);
      const path = [parentPath, name].filter(Boolean).join("/");
      setEntryName("");
      if (selectedSkillId() === skillId) {
        setTree(updatedTree);
        if (entryKind() === "file") await loadFile(skillId, path, true);
        else setSelectedPath(path);
      } else {
        await loadSkills();
      }
    } catch (reason) {
      setError(commandFailure(reason).message);
    }
  }

  function openSkillRename(skill: SkillRecord): void {
    setRenameTarget({ kind: "skill", skillId: skill.id, currentName: skill.name });
    setRenameName(skill.name);
  }

  function openEntryRename(skillId: string, node: SkillTreeNode): void {
    setRenameTarget({
      kind: "entry",
      skillId,
      relativePath: node.relativePath,
      currentName: node.name,
    });
    setRenameName(node.name);
  }

  async function submitRename() {
    const target = renameTarget();
    const newName = renameName().trim();
    if (!target || !newName || newName === target.currentName) return;
    try {
      if (target.kind === "skill") {
        await commands.renameSkill(target.skillId, newName);
        await loadSkills(target.skillId);
      } else {
        const updatedTree = await commands.renameSkillEntry({
          skillId: target.skillId,
          relativePath: target.relativePath,
          newName,
        });
        if (selectedSkillId() === target.skillId) {
          setTree(updatedTree);
          const parent = target.relativePath.split("/").slice(0, -1).join("/");
          const nextPath = [parent, newName].filter(Boolean).join("/");
          const currentPath = selectedPath();
          if (
            currentPath === target.relativePath ||
            currentPath.startsWith(`${target.relativePath}/`)
          ) {
            const remappedPath = `${nextPath}${currentPath.slice(target.relativePath.length)}`;
            if (file()) await loadFile(target.skillId, remappedPath, true);
            else setSelectedPath(remappedPath);
          }
        }
      }
      setRenameTarget();
    } catch (reason) {
      setError(commandFailure(reason).message);
    }
  }

  function requestRemoveSkill(skill: SkillRecord): void {
    setConfirmation({
      title: copy(`删除 Skill “${skill.name}”？`, `Delete Skill “${skill.name}”?`),
      description: copy(
        "这个 Skill 的全部文件会移动到应用内部回收目录。",
        "All files in this Skill will move to the application's internal trash.",
      ),
      confirmLabel: copy("删除 Skill", "Delete Skill"),
      danger: true,
      run: async () => {
        await commands.removeSkill(skill.id);
        if (selectedSkillId() === skill.id) {
          setSelectedSkillId();
          setSelectedPath("");
          setTree();
          setFile();
        }
        await loadSkills();
      },
    });
  }

  function requestRemoveEntry(skillId: string, node: SkillTreeNode): void {
    setConfirmation({
      title: copy(`删除“${node.name}”？`, `Delete “${node.name}”?`),
      description:
        node.kind === "directory"
          ? copy(
              "目录及其中的全部内容会被删除，此操作无法从编辑器中撤销。",
              "The directory and all of its contents will be deleted. This cannot be undone in the editor.",
            )
          : copy(
              "该文件会被删除，此操作无法从编辑器中撤销。",
              "The file will be deleted. This cannot be undone in the editor.",
            ),
      confirmLabel: copy("删除", "Delete"),
      danger: true,
      run: async () => {
        const updatedTree = await commands.removeSkillEntry(skillId, node.relativePath);
        if (selectedSkillId() === skillId) setTree(updatedTree);
        if (
          selectedPath() === node.relativePath ||
          selectedPath().startsWith(`${node.relativePath}/`)
        ) {
          setSelectedPath("");
          setFile();
          setDraft("");
          setDirty(false);
        }
      },
    });
  }

  async function runConfirmation() {
    const request = confirmation();
    if (!request) return;
    setConfirming(true);
    try {
      await request.run();
      setConfirmation();
    } catch (reason) {
      setError(commandFailure(reason).message);
    } finally {
      setConfirming(false);
    }
  }

  function clampSkillListWidth(width: number): number {
    const available = workspaceElement?.clientWidth ?? 900;
    return Math.round(Math.max(220, Math.min(width, Math.max(220, available - 360))));
  }

  function startSkillListResize(event: PointerEvent): void {
    const splitter = event.currentTarget as HTMLElement;
    const workspaceLeft = workspaceElement?.getBoundingClientRect().left ?? 0;
    splitter.setPointerCapture(event.pointerId);
    const update = (next: PointerEvent) => {
      setSkillListWidth(clampSkillListWidth(next.clientX - workspaceLeft));
    };
    const finish = () => {
      splitter.removeEventListener("pointermove", update);
      splitter.removeEventListener("pointerup", finish);
      splitter.removeEventListener("pointercancel", finish);
    };
    splitter.addEventListener("pointermove", update);
    splitter.addEventListener("pointerup", finish);
    splitter.addEventListener("pointercancel", finish);
  }

  function nativeDropTarget(x: number | null, y: number | null): SkillDropTarget | undefined {
    if (x === null || y === null) return undefined;
    const scale = window.devicePixelRatio || 1;
    const element = document
      .elementFromPoint(x / scale, y / scale)
      ?.closest<HTMLElement>("[data-skill-drop-skill-id]");
    const skillId = element?.dataset.skillDropSkillId;
    const parentPath = element?.dataset.skillDropParentPath;
    if (!skillId || parentPath === undefined) return undefined;
    return { skillId, parentPath, key: `${skillId}:${parentPath}` };
  }

  async function handleNativeDrag(payload: SkillNativeDragEvent): Promise<void> {
    if (payload.kind === "leave") {
      setDropActive(false);
      setDropTargetKey();
      return;
    }
    const target = nativeDropTarget(payload.x, payload.y);
    setDropActive(true);
    setDropTargetKey(target?.key);
    if (payload.kind !== "drop") return;
    setDropActive(false);
    setDropTargetKey();
    if (!payload.token) return;
    if (!target) {
      setError(
        copy(
          "请将 .md、.js 或 .py 文件拖到技能列表中的 Skill 或目录上。",
          "Drop .md, .js, or .py files onto a Skill or directory in the Skills list.",
        ),
      );
      return;
    }
    setError();
    try {
      const updatedTree = await commands.importSkillDroppedFiles(
        payload.token,
        target.skillId,
        target.parentPath,
      );
      if (selectedSkillId() === target.skillId) setTree(updatedTree);
      setSkills(await commands.listSkills());
      setDropNotice(
        copy(
          `已导入 ${payload.fileNames.length} 个文件。`,
          `Imported ${payload.fileNames.length} file(s).`,
        ),
      );
    } catch (reason) {
      setError(commandFailure(reason).message);
    }
  }

  onMount(() => {
    void loadSkills()
      .catch((reason) => setError(commandFailure(reason).message))
      .finally(() => setLoading(false));
    void (async () => {
      // eslint-disable-next-line solid/reactivity -- Tauri invokes this callback after mount.
      stopChanges = await listen<SkillChangeEvent[]>("skills:changed", () => {
        void refreshExternalChanges();
      });
      skillSubscriptionId = await commands.subscribeSkills();
    })().catch((reason) => setError(commandFailure(reason).message));
    // eslint-disable-next-line solid/reactivity -- Tauri invokes this callback after mount.
    void listen<SkillNativeDragEvent>("skills:native-drag", (event) => {
      void handleNativeDrag(event.payload);
    })
      .then((stop) => {
        stopNativeDrag = stop;
      })
      .catch((reason) => setError(commandFailure(reason).message));
  });
  onCleanup(() => {
    stopChanges?.();
    stopNativeDrag?.();
    if (skillSubscriptionId) void commands.unsubscribeSkills(skillSubscriptionId);
  });

  return (
    <div class="extension-settings-page" data-testid="skills-settings-page">
      <PageHeading
        class="extension-page-heading"
        title="Skills"
        description={copy(
          "每个一级目录是一个 Skill；入口固定为 SKILL.md，目录内仅支持 .md、.js 和 .py 文件。",
          "Each top-level directory is one Skill. Its entry is SKILL.md and files are limited to .md, .js, and .py.",
        )}
        actions={
          <div class="extension-heading-actions">
            <Button
              data-testid="skill-import"
              disabled={importing()}
              onClick={() => void importSkillArchive()}
            >
              <Upload size={15} />{" "}
              {importing() ? copy("正在导入…", "Importing…") : copy("导入压缩包", "Import ZIP")}
            </Button>
            <Button
              variant="primary"
              data-testid="skill-create"
              onClick={() => setCreateOpen(true)}
            >
              <Plus size={15} /> {copy("新建 Skill", "New Skill")}
            </Button>
          </div>
        }
      />
      <Show when={error()}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <Show when={dropNotice()}>
        {(message) => <StatusBanner tone="success">{message()}</StatusBanner>}
      </Show>
      <Workspace
        ref={workspaceElement}
        class="extension-workspace skills-workspace"
        data-skill-list-width={`${skillListWidth()}px`}
      >
        <aside class="extension-sidebar">
          <div class="extension-panel-toolbar">
            <strong>{copy("技能列表", "Skills")}</strong>
          </div>
          <div class="skill-tree" data-component="skill-tree" data-drop-active={dropActive()}>
            <Show
              when={!loading()}
              fallback={
                <div class="extension-empty">{copy("正在加载 Skills…", "Loading Skills…")}</div>
              }
            >
              <For each={skills()}>
                {(skill) => (
                  <section class="skill-tree-section">
                    <div
                      class="skill-tree-row skill-tree-skill-row"
                      classList={{
                        "skill-is-disabled": !skill.enabled,
                        "skill-drop-target": dropTargetKey() === `${skill.id}:`,
                      }}
                      data-skill-drop-skill-id={skill.editable ? skill.id : undefined}
                      data-skill-drop-parent-path={skill.editable ? "" : undefined}
                    >
                      <Button
                        type="button"
                        class="skill-tree-label"
                        data-testid={`skill-row-${skill.name}`}
                        classList={{ selected: selectedSkillId() === skill.id && !selectedPath() }}
                        onClick={() => void selectSkill(skill.id)}
                      >
                        {selectedSkillId() === skill.id ? (
                          <FolderOpen size={15} />
                        ) : (
                          <Folder size={15} />
                        )}
                        <span title={skill.qualifiedName}>
                          {skill.interface?.displayName ?? skill.qualifiedName}
                        </span>
                        <Show when={!skill.editable}>
                          <span class="skill-disabled-indicator">{skill.scope}</span>
                        </Show>
                        <Show when={skill.treeRevision}>
                          <span
                            class="skill-disabled-indicator"
                            title={`${copy("内容版本", "Content revision")}: ${skill.treeRevision}`}
                          >
                            rev {skill.treeRevision.slice(0, 8)}
                          </span>
                        </Show>
                        <Show when={!skill.enabled}>
                          <span class="skill-disabled-indicator">{copy("已禁用", "Disabled")}</span>
                        </Show>
                      </Button>
                      <div class="skill-tree-row-controls">
                        <div class="skill-tree-row-menu">
                          <Dropdown
                            label={copy(`${skill.name} 的操作`, `Actions for ${skill.name}`)}
                            triggerTestId={`skill-actions-${skill.name}`}
                            actions={[
                              {
                                id: "new-file",
                                label: copy("新建文件", "New file"),
                                testId: `skill-action-new-file-${skill.name}`,
                              },
                              {
                                id: "new-directory",
                                label: copy("新建目录", "New directory"),
                              },
                              {
                                id: "toggle-enabled",
                                label: skill.enabled
                                  ? copy("禁用 Skill", "Disable Skill")
                                  : copy("启用 Skill", "Enable Skill"),
                                testId: `skill-action-toggle-${skill.name}`,
                                separatorBefore: true,
                              },
                              { id: "rename", label: copy("重命名", "Rename") },
                              {
                                id: "remove",
                                label: copy("删除 Skill", "Delete Skill"),
                                danger: true,
                                separatorBefore: true,
                              },
                            ].filter((action) => skill.editable || action.id === "toggle-enabled")}
                            onSelect={(action) => {
                              if (action === "new-file") openEntryDialog(skill.id, "", "file");
                              if (action === "new-directory")
                                openEntryDialog(skill.id, "", "directory");
                              if (action === "toggle-enabled")
                                void setSkillEnabled(skill.id, !skill.enabled);
                              if (action === "rename") openSkillRename(skill);
                              if (action === "remove") requestRemoveSkill(skill);
                            }}
                          >
                            <MoreHorizontal size={16} aria-hidden="true" />
                          </Dropdown>
                        </div>
                      </div>
                    </div>
                    <Show when={selectedSkillId() === skill.id && tree()}>
                      {(root) => (
                        <For each={root().children}>
                          {(node) => (
                            <SkillNode
                              skillId={skill.id}
                              editable={skill.editable}
                              node={node}
                              depth={0}
                              selectedPath={selectedPath()}
                              onSelect={(node) => {
                                if (node.kind === "file")
                                  void loadFile(skill.id, node.relativePath);
                                else setSelectedPath(node.relativePath);
                              }}
                              onCreate={(parentPath, kind) =>
                                openEntryDialog(skill.id, parentPath, kind)
                              }
                              onRename={(node) => openEntryRename(skill.id, node)}
                              onRemove={(node) => requestRemoveEntry(skill.id, node)}
                              activeDropKey={dropTargetKey()}
                              copy={copy}
                            />
                          )}
                        </For>
                      )}
                    </Show>
                  </section>
                )}
              </For>
            </Show>
          </div>
        </aside>
        <div
          class="extension-splitter"
          role="separator"
          tabIndex={0}
          aria-label={copy("调整技能列表宽度", "Resize Skill list")}
          aria-orientation="vertical"
          aria-valuemin={220}
          aria-valuemax={Math.max(220, (workspaceElement?.clientWidth ?? 900) - 360)}
          aria-valuenow={skillListWidth()}
          onPointerDown={startSkillListResize}
          onKeyDown={(event) => {
            if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
            event.preventDefault();
            const direction = event.key === "ArrowLeft" ? -1 : 1;
            setSkillListWidth((width) => clampSkillListWidth(width + direction * 20));
          }}
        />
        <main class="extension-main">
          <Show
            when={file()}
            fallback={
              <div class="extension-empty">
                <Show
                  when={(selectedSkill()?.diagnostics.length ?? 0) > 0}
                  fallback={
                    <>
                      {copy(
                        "选择一个 Skill 文件开始编辑。",
                        "Select a Skill file to start editing.",
                      )}
                    </>
                  }
                >
                  <div class="skill-invalid-summary">
                    <strong>{copy("这个 Skill 需要修复", "This Skill needs attention")}</strong>
                    <For each={selectedSkill()?.diagnostics ?? []}>
                      {(diagnostic) => <span>{diagnostic.message}</span>}
                    </For>
                    <span>
                      {copy(
                        "可在技能列表中新建 SKILL.md；系统不会自动修改外部目录。",
                        "Create SKILL.md from the Skill list. Hachimi never modifies external directories automatically.",
                      )}
                    </span>
                  </div>
                </Show>
              </div>
            }
          >
            {(snapshot) => (
              <TextEditor
                path={snapshot().relativePath}
                kind={snapshot().editorKind}
                value={draft()}
                dirty={dirty()}
                saving={saving()}
                readOnly={!selectedSkill()?.editable}
                conflict={conflict()}
                diagnostics={snapshot().diagnostics}
                onInput={(value) => {
                  setDraft(value);
                  setDirty(value !== (snapshot().content ?? ""));
                }}
                onSave={() => void save()}
                onReload={() => void loadFile(snapshot().skillId, snapshot().relativePath, true)}
                onKeepLocal={() => void keepLocalDraft()}
                resolveReference={resolvePreviewReference}
                referenceFiles={referenceDestinations(tree(), snapshot().relativePath)}
              />
            )}
          </Show>
        </main>
      </Workspace>

      <Dialog
        open={createOpen()}
        title={copy("新建 Skill", "New Skill")}
        onOpenChange={setCreateOpen}
        closeLabel={copy("关闭", "Close")}
      >
        <div class="dialog-form">
          <TextField
            label={copy("Skill 名称", "Skill name")}
            value={newSkillName()}
            placeholder={copy("例如 release-notes", "For example, release-notes")}
            description={copy(
              "仅支持小写字母、数字和连字符，长度 1–64。",
              "Use 1–64 lowercase ASCII letters, digits, or hyphens.",
            )}
            onInput={(event) => setNewSkillName(event.currentTarget.value)}
          />
          <div class="dialog-actions">
            <Button onClick={() => setCreateOpen(false)}>{copy("取消", "Cancel")}</Button>
            <Button
              variant="primary"
              disabled={!newSkillName().trim()}
              onClick={() => void createSkill()}
            >
              {copy("创建", "Create")}
            </Button>
          </div>
        </div>
      </Dialog>

      <Dialog
        open={entryOpen()}
        title={
          entryKind() === "file" ? copy("新建文件", "New file") : copy("新建目录", "New directory")
        }
        description={
          entryParentPath()
            ? copy(
                `将在 ${entryParentPath()} 中创建。`,
                `The entry will be created in ${entryParentPath()}.`,
              )
            : copy("将在 Skill 根目录中创建。", "The entry will be created in the Skill root.")
        }
        onOpenChange={setEntryOpen}
        closeLabel={copy("关闭", "Close")}
      >
        <div class="dialog-form">
          <TextField
            label={copy("名称", "Name")}
            testId="skill-entry-name"
            value={entryName()}
            placeholder={entryKind() === "file" ? "reference.md" : "references"}
            {...(entryKind() === "file"
              ? {
                  description: copy(
                    "文件仅支持 .md、.js 和 .py。",
                    "Files must use .md, .js, or .py.",
                  ),
                }
              : {})}
            onInput={(event) => setEntryName(event.currentTarget.value)}
          />
          <div class="dialog-actions">
            <Button onClick={() => setEntryOpen(false)}>{copy("取消", "Cancel")}</Button>
            <Button
              variant="primary"
              disabled={!entryName().trim()}
              onClick={() => void createEntry()}
            >
              {copy("创建", "Create")}
            </Button>
          </div>
        </div>
      </Dialog>

      <Dialog
        open={Boolean(renameTarget())}
        title={
          renameTarget()?.kind === "skill"
            ? copy("重命名 Skill", "Rename Skill")
            : copy("重命名文件或目录", "Rename file or directory")
        }
        description={copy(
          "请输入新的名称。路径引用不会自动改写。",
          "Enter a new name. Path references are not rewritten automatically.",
        )}
        onOpenChange={(open) => {
          if (!open) setRenameTarget();
        }}
        closeLabel={copy("关闭", "Close")}
      >
        <div class="dialog-form">
          <TextField
            label={copy("新名称", "New name")}
            value={renameName()}
            onInput={(event) => setRenameName(event.currentTarget.value)}
          />
          <div class="dialog-actions">
            <Button onClick={() => setRenameTarget()}>{copy("取消", "Cancel")}</Button>
            <Button
              variant="primary"
              disabled={!renameName().trim() || renameName().trim() === renameTarget()?.currentName}
              onClick={() => void submitRename()}
            >
              {copy("重命名", "Rename")}
            </Button>
          </div>
        </div>
      </Dialog>

      <Dialog
        open={Boolean(confirmation())}
        title={confirmation()?.title ?? ""}
        description={confirmation()?.description ?? ""}
        onOpenChange={(open) => {
          if (!open && !confirming()) setConfirmation();
        }}
        closeLabel={copy("关闭", "Close")}
      >
        <div class="dialog-confirmation-actions">
          <Button disabled={confirming()} onClick={() => setConfirmation()}>
            {copy("取消", "Cancel")}
          </Button>
          <Button
            variant={confirmation()?.danger ? "danger" : "primary"}
            disabled={confirming()}
            onClick={() => void runConfirmation()}
          >
            {confirmation()?.confirmLabel}
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

function SkillNode(props: {
  skillId: string;
  editable: boolean;
  node: SkillTreeNode;
  depth: number;
  selectedPath: string;
  onSelect: (node: SkillTreeNode) => void;
  onCreate: (parentPath: string, kind: SkillEntryKind) => void;
  onRename: (node: SkillTreeNode) => void;
  onRemove: (node: SkillTreeNode) => void;
  activeDropKey: string | undefined;
  copy: (zh: string, en: string) => string;
}) {
  const actions = createMemo(() => {
    const result: DropdownAction[] = [];
    if (!props.editable) return result;
    if (props.node.kind === "directory") {
      result.push(
        { id: "new-file", label: props.copy("新建文件", "New file") },
        { id: "new-directory", label: props.copy("新建目录", "New directory") },
      );
    }
    if (props.node.relativePath !== "SKILL.md") {
      result.push({ id: "rename", label: props.copy("重命名", "Rename") });
      result.push({
        id: "remove",
        label: props.copy("删除", "Delete"),
        danger: true,
        separatorBefore: props.node.kind === "directory",
      });
    }
    return result;
  });

  return (
    <div class="skill-tree-node" data-component="skill-tree-node" data-tree-depth={props.depth}>
      <div
        class="skill-tree-row"
        data-component="skill-tree-row"
        classList={{
          "skill-drop-target":
            props.node.kind === "directory" &&
            props.activeDropKey === `${props.skillId}:${props.node.relativePath}`,
        }}
        {...(props.editable && props.node.kind === "directory"
          ? {
              "data-skill-drop-skill-id": props.skillId,
              "data-skill-drop-parent-path": props.node.relativePath,
            }
          : {})}
      >
        <Button
          type="button"
          class="skill-tree-label"
          data-testid={`skill-node-${props.node.relativePath.replaceAll("/", "--")}`}
          classList={{ selected: props.selectedPath === props.node.relativePath }}
          onClick={() => props.onSelect(props.node)}
        >
          {props.node.kind === "directory" ? (
            <Folder size={14} />
          ) : props.node.editorKind === "markdown" ? (
            <FileText size={14} />
          ) : (
            <Code2 size={14} />
          )}
          <span>{props.node.name}</span>
          <Show when={props.node.editorKind === "unsupported"}>
            <span class="node-meta">{props.copy("只读", "Read only")}</span>
          </Show>
        </Button>
        <Show when={actions().length > 0}>
          <div class="skill-tree-row-menu">
            <Dropdown
              label={props.copy(`${props.node.name} 的操作`, `Actions for ${props.node.name}`)}
              triggerTestId={`skill-actions-${props.node.relativePath.replaceAll("/", "--")}`}
              actions={actions()}
              onSelect={(action) => {
                if (action === "new-file") props.onCreate(props.node.relativePath, "file");
                if (action === "new-directory")
                  props.onCreate(props.node.relativePath, "directory");
                if (action === "rename") props.onRename(props.node);
                if (action === "remove") props.onRemove(props.node);
              }}
            >
              <MoreHorizontal size={16} aria-hidden="true" />
            </Dropdown>
          </div>
        </Show>
      </div>
      <For each={props.node.children}>
        {(child) => (
          <SkillNode
            skillId={props.skillId}
            editable={props.editable}
            node={child}
            depth={props.depth + 1}
            selectedPath={props.selectedPath}
            onSelect={props.onSelect}
            onCreate={props.onCreate}
            onRename={props.onRename}
            onRemove={props.onRemove}
            activeDropKey={props.activeDropKey}
            copy={props.copy}
          />
        )}
      </For>
    </div>
  );
}

function referenceDestinations(root: SkillTreeNode | undefined, sourcePath: string): string[] {
  if (!root) return [];
  const parent = sourcePath.split("/").slice(0, -1).join("/");
  const prefix = parent ? `${parent}/` : "";
  const files: string[] = [];
  const visit = (node: SkillTreeNode) => {
    if (node.kind === "file" && node.relativePath !== sourcePath) {
      if (!prefix || node.relativePath.startsWith(prefix)) {
        files.push(prefix ? node.relativePath.slice(prefix.length) : node.relativePath);
      }
    }
    node.children.forEach(visit);
  };
  visit(root);
  return files.sort((left, right) => left.localeCompare(right));
}

function findNode(root: SkillTreeNode | undefined, path: string): SkillTreeNode | undefined {
  if (!root) return undefined;
  if (root.relativePath === path) return root;
  for (const child of root.children) {
    const found = findNode(child, path);
    if (found) return found;
  }
  return undefined;
}
