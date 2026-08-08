import {
  commandFailure,
  type DiffReadFileResponse,
  type DiffScope,
  type FileDiffSummary,
  type FsEntry,
  type FsFileChunk,
  type FsWatchRegistration,
  type GitWorkspaceSnapshot,
  type RunDiffSnapshot,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  AlertTriangle,
  Button,
  Check,
  ChevronDown,
  File,
  FolderOpen,
  GitBranch,
  GitFork,
  RefreshCw,
  SearchField,
  SelectField,
  X,
} from "@hachimi/ui";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, untrack } from "solid-js";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { runMutationContext } from "./mutation-context";
import { WorkspaceFileEditor } from "./workspace-file-editor";

const CHUNK_BYTES = 256 * 1024;
const MAX_EDIT_BYTES = 2 * 1024 * 1024;

export function WorkspaceBrowser(props: {
  mode: "files" | "review";
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  initialPath?: string;
  initialDiffRunId?: string;
  initialDiffBaseBranch?: string;
  diffBranches?: readonly string[];
  initialDiffScope?: "run" | "checkout" | "branch" | "session";
}) {
  const i18n = useI18n();
  const [entries, setEntries] = createSignal<FsEntry[]>([]);
  const [expanded, setExpanded] = createSignal<Record<string, FsEntry[]>>({});
  const [selectedPath, setSelectedPath] = createSignal<string>();
  const [chunks, setChunks] = createSignal<FsFileChunk[]>([]);
  const [savedText, setSavedText] = createSignal("");
  const [draft, setDraft] = createSignal("");
  const [fileEtag, setFileEtag] = createSignal<string>();
  const [saveConflict, setSaveConflict] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [treeFilter, setTreeFilter] = createSignal("");
  const [diffScope, setDiffScope] = createSignal<"run" | "checkout" | "branch" | "session">(
    untrack(() => props.initialDiffScope ?? (props.initialDiffBaseBranch ? "branch" : "checkout")),
  );
  const [diffBaseBranch, setDiffBaseBranch] = createSignal(
    untrack(() => props.initialDiffBaseBranch ?? ""),
  );
  const [diff, setDiff] = createSignal<RunDiffSnapshot>();
  const [git, setGit] = createSignal<GitWorkspaceSnapshot>();
  const [selectedDiffPath, setSelectedDiffPath] = createSignal<string>();
  const [diffFilter, setDiffFilter] = createSignal("");
  const [collapsedDiffFolders, setCollapsedDiffFolders] = createSignal<Set<string>>(new Set());
  const [diffFileChunks, setDiffFileChunks] = createSignal<Record<string, DiffReadFileResponse[]>>(
    {},
  );
  const [watchInvalidated, setWatchInvalidated] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  let watch: FsWatchRegistration | undefined;
  let diffRequest = 0;
  let fileRequest = 0;

  const latestRun = createMemo(() => props.snapshot.runs[props.snapshot.runs.length - 1]);
  const dirty = createMemo(() => draft() !== savedText());
  const firstChunk = createMemo(() => chunks()[0]);
  const lastChunk = createMemo(() => chunks()[chunks().length - 1]);
  const fileEditable = createMemo(
    () =>
      Boolean(firstChunk()) &&
      !firstChunk()?.binary &&
      firstChunk()!.byteSize <= MAX_EDIT_BYTES &&
      lastChunk()?.eof === true,
  );
  const checkoutId = createMemo(() =>
    props.snapshot.session.context.kind === "project"
      ? props.snapshot.session.context.checkout_id
      : undefined,
  );
  const context = () => ({
    sessionId: props.snapshot.session.id,
    checkoutId: checkoutId() ?? "",
  });
  const currentScope = (): DiffScope | undefined => {
    const run = props.initialDiffRunId
      ? props.snapshot.runs.find((candidate) => candidate.id === props.initialDiffRunId)
      : latestRun();
    if (diffScope() === "run" && run) return { kind: "run", run_id: run.id };
    const currentCheckoutId = checkoutId();
    if (diffScope() === "session" && currentCheckoutId) {
      return {
        kind: "session",
        session_id: props.snapshot.session.id,
        checkout_id: currentCheckoutId,
      };
    }
    if (diffScope() === "branch" && currentCheckoutId && diffBaseBranch()) {
      return {
        kind: "branch",
        checkout_id: currentCheckoutId,
        branch: diffBaseBranch(),
      };
    }
    return currentCheckoutId ? { kind: "checkout", checkout_id: currentCheckoutId } : undefined;
  };
  const chunksForDiff = (path: string) => diffFileChunks()[path] ?? [];
  const selectedDiffFile = createMemo(() => {
    const snapshot = diff();
    return snapshot?.files.find((file) => file.path === selectedDiffPath()) ?? snapshot?.files[0];
  });
  const diffTotals = createMemo(() =>
    (diff()?.files ?? []).reduce(
      (totals, file) => ({
        additions: totals.additions + file.additions,
        deletions: totals.deletions + file.deletions,
      }),
      { additions: 0, deletions: 0 },
    ),
  );
  const diffBranchOptions = createMemo(() =>
    [...new Set([diffBaseBranch(), ...(props.diffBranches ?? [])])]
      .filter(Boolean)
      .map((branch) => ({ value: branch, label: branch })),
  );
  const diffTree = createMemo(() => {
    const filter = diffFilter().trim().toLocaleLowerCase();
    return buildDiffTree(
      (diff()?.files ?? []).filter((file) => file.path.toLocaleLowerCase().includes(filter)),
    );
  });
  const lastDiffChunk = (path: string) => {
    const chunks = chunksForDiff(path);
    return chunks[chunks.length - 1];
  };

  async function loadDirectory(
    path = "",
    append = false,
    cursor: string | null = null,
    acknowledgeInvalidation = false,
  ) {
    setBusy(true);
    try {
      const page = await props.commandPort.listWorkspaceFiles({
        ...context(),
        path,
        cursor,
        limit: 500,
      });
      if (path) {
        setExpanded((current) => ({
          ...current,
          [path]: append ? [...(current[path] ?? []), ...page.entries] : page.entries,
        }));
      } else {
        setEntries((current) => (append ? [...current, ...page.entries] : page.entries));
      }
      if (acknowledgeInvalidation) setWatchInvalidated(false);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function openFile(path: string, preserveDraft = false) {
    const request = ++fileRequest;
    const localDraft = preserveDraft ? untrack(draft) : undefined;
    setSelectedPath(path);
    setChunks([]);
    setSaveConflict(false);
    setBusy(true);
    try {
      const loaded: FsFileChunk[] = [];
      let offset = 0;
      let etag: string | null = null;
      while (true) {
        const chunk = await props.commandPort.readWorkspaceFileChunk({
          ...context(),
          path,
          offset,
          limit: CHUNK_BYTES,
          ifMatch: etag,
        });
        if (!chunk) throw new Error("Workspace file read returned an empty response.");
        if (request !== fileRequest || untrack(selectedPath) !== path) return;
        loaded.push(chunk);
        etag ??= chunk.etag;
        if (chunk.eof || chunk.binary || chunk.byteSize > MAX_EDIT_BYTES) break;
        offset = chunk.nextOffset;
      }
      const text = loaded.map((chunk) => chunk.utf8Text ?? "").join("");
      setChunks(loaded);
      setFileEtag(etag ?? undefined);
      setSavedText(text);
      setDraft(localDraft ?? text);
    } catch (error) {
      if (request === fileRequest) setFailure(commandFailure(error).message);
    } finally {
      if (request === fileRequest) setBusy(false);
    }
  }

  function closeFile() {
    fileRequest += 1;
    setSelectedPath(undefined);
    setChunks([]);
    setSavedText("");
    setDraft("");
    setFileEtag(undefined);
    setSaveConflict(false);
  }

  async function loadNextFileChunk(path: string) {
    const last = lastChunk();
    if (!last || last.eof) return;
    const request = fileRequest;
    setBusy(true);
    try {
      const chunk = await props.commandPort.readWorkspaceFileChunk({
        ...context(),
        path,
        offset: last.nextOffset,
        limit: CHUNK_BYTES,
        ifMatch: fileEtag() ?? null,
      });
      if (!chunk) throw new Error("Workspace file read returned an empty response.");
      if (request !== fileRequest || untrack(selectedPath) !== path) return;
      setChunks((current) => [...current, chunk]);
      setSavedText((current) => current + (chunk.utf8Text ?? ""));
      setDraft((current) => current + (chunk.utf8Text ?? ""));
    } catch (error) {
      if (request === fileRequest) setFailure(commandFailure(error).message);
    } finally {
      if (request === fileRequest) setBusy(false);
    }
  }

  async function saveFile() {
    const path = selectedPath();
    const run = latestRun();
    const etag = fileEtag();
    const currentCheckoutId = checkoutId();
    if (!path || !run || !etag || !currentCheckoutId || !fileEditable() || !dirty()) return;
    setSaving(true);
    try {
      const response = await props.commandPort.writeWorkspaceFile({
        context: runMutationContext(run),
        sessionId: props.snapshot.session.id,
        checkoutId: currentCheckoutId,
        path,
        content: draft(),
        ifMatch: etag,
      });
      setFileEtag(response.etag);
      setChunks((current) => current.map((chunk) => ({ ...chunk, etag: response.etag })));
      setSavedText(untrack(draft));
      setSaveConflict(false);
    } catch (error) {
      const failure = commandFailure(error);
      if (
        failure.code === "workspace_conflict" ||
        failure.code === "workspace_write_indeterminate"
      ) {
        setSaveConflict(true);
      } else {
        setFailure(failure.message);
      }
    } finally {
      setSaving(false);
    }
  }

  async function loadDiff() {
    const scope = currentScope();
    if (!scope) return;
    const request = ++diffRequest;
    const scopeKey = JSON.stringify(scope);
    setBusy(true);
    try {
      const snapshot = await props.commandPort.getWorkspaceDiff(scope);
      if (request === diffRequest && JSON.stringify(currentScope()) === scopeKey) {
        setDiff(snapshot);
        setDiffFileChunks({});
        setSelectedDiffPath((current) => {
          if (current && snapshot.files.some((file) => file.path === current)) return current;
          if (props.initialPath && snapshot.files.some((file) => file.path === props.initialPath))
            return props.initialPath;
          return snapshot.files[0]?.path;
        });
      }
    } catch (error) {
      if (request === diffRequest) setFailure(commandFailure(error).message);
    } finally {
      if (request === diffRequest) setBusy(false);
    }
  }

  async function loadGit() {
    const currentCheckoutId = checkoutId();
    if (!currentCheckoutId) return;
    try {
      setGit(
        await props.commandPort.getWorkspaceGit({
          sessionId: props.snapshot.session.id,
          checkoutId: currentCheckoutId,
          historyLimit: 1,
        }),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  async function loadDiffFile(path: string, offset = 0, etag?: string) {
    const scope = currentScope();
    if (!scope) return;
    const request = diffRequest;
    const scopeKey = JSON.stringify(scope);
    setBusy(true);
    try {
      const chunk = await props.commandPort.readWorkspaceDiffFile({
        scope,
        path,
        offset,
        limit: CHUNK_BYTES,
        ifMatch: etag ?? null,
      });
      if (!chunk) throw new Error("Workspace diff read returned an empty response.");
      if (request !== diffRequest || JSON.stringify(currentScope()) !== scopeKey) return;
      setDiffFileChunks((current) => ({
        ...current,
        [path]: offset === 0 ? [chunk] : [...(current[path] ?? []), chunk],
      }));
    } catch (error) {
      if (request === diffRequest) setFailure(commandFailure(error).message);
    } finally {
      if (request === diffRequest) setBusy(false);
    }
  }

  async function toggleDirectory(entry: FsEntry) {
    if (expanded()[entry.path]) {
      setExpanded((current) => {
        const next = { ...current };
        delete next[entry.path];
        return next;
      });
      return;
    }
    await loadDirectory(entry.path);
  }

  createEffect(() => {
    setDiffBaseBranch(props.initialDiffBaseBranch ?? "");
    if (props.initialDiffScope) setDiffScope(props.initialDiffScope);
    else if (props.initialDiffBaseBranch) setDiffScope("branch");
  });

  createEffect(() => {
    const sessionId = props.snapshot.session.id;
    const currentCheckoutId = checkoutId();
    const commandPort = props.commandPort;
    if (!currentCheckoutId) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    setEntries([]);
    setExpanded({});
    setSelectedPath(undefined);
    setChunks([]);
    setSavedText("");
    setDraft("");
    setFileEtag(undefined);
    setSaveConflict(false);
    diffRequest += 1;
    setDiff(undefined);
    setSelectedDiffPath(undefined);
    setDiffFileChunks({});
    setWatchInvalidated(false);
    if (props.mode === "files") void loadDirectory();
    if (props.mode === "review") void loadGit();
    void commandPort
      .watchWorkspaceFiles({
        sessionId,
        checkoutId: currentCheckoutId,
        path: "",
        recursive: true,
      })
      .then(async (registration) => {
        if (disposed) {
          await commandPort.unwatchWorkspaceFiles(registration.id).catch(() => false);
          return;
        }
        watch = registration;
        // eslint-disable-next-line solid/reactivity -- workspace events are delivered after the effect setup.
        unlisten = await commandPort.onWorkspaceChange((event) => {
          if (event.watchId !== registration.id || event.generation !== registration.generation) {
            return;
          }
          if (event.kind === "invalidated" || event.overflowed) setWatchInvalidated(true);
          if (props.mode === "files") void loadDirectory();
          if (props.mode === "review") {
            void loadDiff();
            void loadGit();
          }
          const openPath = untrack(selectedPath);
          if (openPath && (event.paths.length === 0 || event.paths.includes(openPath))) {
            if (untrack(dirty) && !untrack(saving)) setSaveConflict(true);
            else if (!untrack(saving)) void openFile(openPath);
          }
        });
      })
      .catch((error) => setFailure(commandFailure(error).message));
    onCleanup(() => {
      disposed = true;
      unlisten?.();
      if (watch) void commandPort.unwatchWorkspaceFiles(watch.id).catch(() => false);
      watch = undefined;
    });
  });

  createEffect(() => {
    const sessionId = props.snapshot.session.id;
    const path = props.initialPath;
    if (!path || !sessionId) return;
    if (props.mode === "review") {
      if (props.initialDiffRunId) setDiffScope("run");
      return;
    }
    void openFile(path);
  });

  createEffect(() => {
    const mode = props.mode;
    diffScope();
    diffBaseBranch();
    if (mode === "review") void loadDiff();
  });

  return (
    <aside
      class="workspace-browser"
      data-mode={props.mode}
      data-component="workspace"
      aria-label={i18n.locale() === "zh-CN" ? "工作区" : "Workspace"}
    >
      <header class="workspace-mode-header">
        <Show
          when={props.mode === "review"}
          fallback={
            <div class="workspace-file-tab-title">
              <File size={14} />
              <strong>
                {selectedPath()?.split(/[\\/]/).at(-1) ??
                  (i18n.locale() === "zh-CN" ? "文件" : "Files")}
              </strong>
              <Show when={dirty()}>
                <span
                  class="workspace-file-dirty"
                  aria-label={i18n.locale() === "zh-CN" ? "未保存" : "Unsaved"}
                />
              </Show>
              <Show when={selectedPath() && fileEditable()}>
                <Button
                  data-testid="workspace-save-file"
                  disabled={!dirty() || saving() || saveConflict()}
                  title={i18n.locale() === "zh-CN" ? "保存 (Ctrl+S)" : "Save (Ctrl+S)"}
                  onClick={() => void saveFile()}
                >
                  <Check size={13} />
                </Button>
              </Show>
              <Show when={selectedPath()}>
                <Button
                  aria-label={i18n.locale() === "zh-CN" ? "关闭文件" : "Close file"}
                  title={i18n.locale() === "zh-CN" ? "关闭文件" : "Close file"}
                  onClick={closeFile}
                >
                  <X size={13} />
                </Button>
              </Show>
            </div>
          }
        >
          <div class="workspace-review-heading">
            <strong>{i18n.locale() === "zh-CN" ? "分支" : "Branch"}</strong>
            <span>{git()?.branch ?? "HEAD"}</span>
            <Show when={diffScope() === "branch" && diffBranchOptions().length > 0}>
              <span aria-hidden="true">→</span>
              <SelectField
                label={i18n.locale() === "zh-CN" ? "比较分支" : "Compare branch"}
                testId="workspace-diff-branch-select"
                size="small"
                value={diffBaseBranch()}
                options={diffBranchOptions()}
                onChange={setDiffBaseBranch}
              />
            </Show>
            <b class="diff-additions">+{diffTotals().additions}</b>
            <b class="diff-deletions">-{diffTotals().deletions}</b>
          </div>
        </Show>
        <Button
          class="workspace-refresh"
          aria-label={i18n.locale() === "zh-CN" ? "刷新工作区" : "Refresh workspace"}
          title={i18n.locale() === "zh-CN" ? "刷新工作区" : "Refresh workspace"}
          disabled={busy()}
          onClick={() => {
            if (props.mode === "review") {
              void loadDiff();
              void loadGit();
            } else {
              void loadDirectory("", false, null, true);
            }
          }}
        >
          <RefreshCw size={13} />
        </Button>
      </header>
      <Show when={watchInvalidated()}>
        <div class="workspace-invalidation" role="status">
          <AlertTriangle size={13} />
          {i18n.locale() === "zh-CN" ? "文件变化过多，已重新加载" : "Changes invalidated; reloaded"}
        </div>
      </Show>
      <Show when={failure()}>
        {(message) => (
          <div class="workspace-browser-error">
            <span>{message()}</span>
            <Button
              aria-label={i18n.locale() === "zh-CN" ? "关闭错误" : "Dismiss error"}
              title={i18n.locale() === "zh-CN" ? "关闭错误" : "Dismiss error"}
              onClick={() => setFailure(undefined)}
            >
              <X size={12} />
            </Button>
          </div>
        )}
      </Show>
      <Show when={props.mode === "files"}>
        <div class="workspace-path-bar">
          <span>/</span>
          <span>{selectedPath() ?? ""}</span>
        </div>
        <div class="workspace-files-layout">
          <div class="workspace-file-content">
            <Show
              when={selectedPath()}
              fallback={
                <div class="inspector-empty-state workspace-file-empty">
                  <FolderOpen size={34} />
                  <strong>{i18n.locale() === "zh-CN" ? "打开文件" : "Open a file"}</strong>
                  <span>
                    {i18n.locale() === "zh-CN"
                      ? "从右侧工作区目录树中选择文件"
                      : "Choose a file from the workspace tree"}
                  </span>
                </div>
              }
            >
              {(path) => (
                <>
                  <WorkspaceFileEditor
                    path={path()}
                    value={draft()}
                    editable={fileEditable()}
                    dirty={dirty()}
                    saving={saving()}
                    conflict={saveConflict()}
                    showHeader={false}
                    workspaceRoot={props.snapshot.checkout?.path}
                    locale={i18n.locale()}
                    readOnlyMessage={
                      firstChunk()?.binary
                        ? i18n.locale() === "zh-CN"
                          ? "二进制文件仅按受控分块读取。"
                          : "Binary files remain read-only and are loaded in controlled chunks."
                        : firstChunk()?.byteSize && firstChunk()!.byteSize > MAX_EDIT_BYTES
                          ? i18n.locale() === "zh-CN"
                            ? "超过 2 MiB 的文件保持只读，避免一次性进入前端状态。"
                            : "Files larger than 2 MiB remain read-only and are not loaded into editor state at once."
                          : busy()
                            ? i18n.locale() === "zh-CN"
                              ? "正在加载文件…"
                              : "Loading file…"
                            : undefined
                    }
                    onInput={setDraft}
                    onSave={() => void saveFile()}
                    onReload={() => void openFile(path())}
                    onKeepLocal={() => void openFile(path(), true)}
                    onClose={closeFile}
                    onOpenPath={(target) => void openFile(target)}
                  />
                  <Show when={chunks().length > 0 && lastChunk()?.eof === false}>
                    <Button
                      class="workspace-load-more"
                      disabled={busy()}
                      onClick={() => void loadNextFileChunk(path())}
                    >
                      {i18n.locale() === "zh-CN" ? "加载下一分块" : "Load next chunk"}
                    </Button>
                  </Show>
                </>
              )}
            </Show>
          </div>
          <div class="workspace-file-sidebar">
            <SearchField
              label={i18n.locale() === "zh-CN" ? "筛选文件" : "Filter files"}
              placeholder={i18n.locale() === "zh-CN" ? "筛选文件…" : "Filter files…"}
              value={treeFilter()}
              onInput={(event) => setTreeFilter(event.currentTarget.value)}
            />
            <div class="workspace-file-tree" data-component="file-tree">
              <For
                each={entries().filter((entry) =>
                  treeEntryMatches(entry, treeFilter(), expanded()),
                )}
              >
                {(entry) => (
                  <TreeEntry
                    entry={entry}
                    depth={0}
                    expanded={expanded()}
                    filter={treeFilter()}
                    onDirectory={toggleDirectory}
                    onFile={(path) => void openFile(path)}
                  />
                )}
              </For>
            </div>
          </div>
        </div>
      </Show>
      <Show when={props.mode === "review"}>
        <div class="workspace-diff-panel" data-component="diff">
          <div class="workspace-diff-layout">
            <div class="workspace-diff-content">
              <Show
                when={selectedDiffFile()}
                fallback={
                  <div class="inspector-empty-state">
                    <GitBranch size={32} />
                    <strong>
                      {i18n.locale() === "zh-CN" ? "没有 Git 变更" : "No Git changes"}
                    </strong>
                  </div>
                }
              >
                {(file) => (
                  <>
                    <header class="workspace-diff-file-header">
                      <strong>{file().path}</strong>
                      <small>
                        +{file().additions} −{file().deletions}
                      </small>
                    </header>
                    <div class="workspace-diff-scroll">
                      <Show
                        when={!file().binary && !file().tooLarge}
                        fallback={
                          <Show when={!file().binary} fallback={<p>Binary file</p>}>
                            <div class="workspace-diff-artifact">
                              <Show
                                when={chunksForDiff(file().path).length > 0}
                                fallback={
                                  <Button
                                    disabled={busy()}
                                    onClick={() => void loadDiffFile(file().path)}
                                  >
                                    {i18n.locale() === "zh-CN" ? "加载完整 Diff" : "Load full Diff"}
                                  </Button>
                                }
                              >
                                <pre>
                                  {chunksForDiff(file().path)
                                    .map((chunk) => chunk.utf8Text ?? "")
                                    .join("")}
                                </pre>
                                <Show when={lastDiffChunk(file().path)?.eof === false}>
                                  <Button
                                    disabled={busy()}
                                    onClick={() => {
                                      const last = lastDiffChunk(file().path);
                                      if (last)
                                        void loadDiffFile(file().path, last.nextOffset, last.etag);
                                    }}
                                  >
                                    {i18n.locale() === "zh-CN" ? "加载下一分块" : "Load next chunk"}
                                  </Button>
                                </Show>
                              </Show>
                            </div>
                          </Show>
                        }
                      >
                        <For each={file().hunks}>
                          {(hunk) => (
                            <div class="workspace-diff-hunk">
                              <code>{hunk.header}</code>
                              <For each={hunk.lines}>
                                {(line) => (
                                  <pre class={`diff-${line.kind}`}>
                                    {line.kind === "addition"
                                      ? "+"
                                      : line.kind === "deletion"
                                        ? "-"
                                        : " "}
                                    {line.text}
                                  </pre>
                                )}
                              </For>
                            </div>
                          )}
                        </For>
                      </Show>
                    </div>
                  </>
                )}
              </Show>
            </div>
            <aside class="workspace-diff-sidebar">
              <SearchField
                label={i18n.locale() === "zh-CN" ? "筛选变更文件" : "Filter changed files"}
                placeholder={i18n.locale() === "zh-CN" ? "筛选文件…" : "Filter files…"}
                value={diffFilter()}
                onInput={(event) => setDiffFilter(event.currentTarget.value)}
              />
              <div class="workspace-diff-file-list">
                <For each={diffTree()}>
                  {(entry) => (
                    <DiffTreeEntry
                      entry={entry}
                      depth={0}
                      selectedPath={selectedDiffFile()?.path}
                      collapsed={collapsedDiffFolders()}
                      onToggle={(path) => {
                        setCollapsedDiffFolders((current) => {
                          const next = new Set(current);
                          if (next.has(path)) next.delete(path);
                          else next.add(path);
                          return next;
                        });
                      }}
                      onFile={(path) => setSelectedDiffPath(path)}
                    />
                  )}
                </For>
              </div>
            </aside>
          </div>
        </div>
      </Show>
    </aside>
  );
}

type DiffTreeNode =
  | { kind: "directory"; name: string; path: string; children: DiffTreeNode[] }
  | { kind: "file"; name: string; path: string; file: FileDiffSummary };

function buildDiffTree(files: FileDiffSummary[]): DiffTreeNode[] {
  const root: DiffTreeNode[] = [];
  for (const file of files) {
    const parts = file.path.split("/").filter(Boolean);
    let children = root;
    let parent = "";
    for (const [index, name] of parts.entries()) {
      const path = parent ? `${parent}/${name}` : name;
      if (index === parts.length - 1) {
        children.push({ kind: "file", name, path: file.path, file });
        break;
      }
      let directory = children.find(
        (entry): entry is Extract<DiffTreeNode, { kind: "directory" }> =>
          entry.kind === "directory" && entry.path === path,
      );
      if (!directory) {
        directory = { kind: "directory", name, path, children: [] };
        children.push(directory);
      }
      children = directory.children;
      parent = path;
    }
  }
  const sort = (nodes: DiffTreeNode[]) => {
    nodes.sort((left, right) => {
      if (left.kind !== right.kind) return left.kind === "directory" ? -1 : 1;
      return left.name.localeCompare(right.name);
    });
    for (const node of nodes) if (node.kind === "directory") sort(node.children);
  };
  sort(root);
  return root;
}

function DiffTreeEntry(props: {
  entry: DiffTreeNode;
  depth: number;
  selectedPath: string | undefined;
  collapsed: Set<string>;
  onToggle: (path: string) => void;
  onFile: (path: string) => void;
}) {
  return (
    <>
      <Button
        class="workspace-diff-tree-entry"
        classList={{
          active: props.entry.kind === "file" && props.selectedPath === props.entry.path,
        }}
        style={{ "--tree-depth": props.depth }}
        data-status={props.entry.kind === "file" ? props.entry.file.status : undefined}
        onClick={() =>
          props.entry.kind === "directory"
            ? props.onToggle(props.entry.path)
            : props.onFile(props.entry.path)
        }
      >
        {props.entry.kind === "directory" ? (
          <ChevronDown size={13} classList={{ collapsed: props.collapsed.has(props.entry.path) }} />
        ) : (
          <GitFork size={13} />
        )}
        <span>{props.entry.name}</span>
        <Show when={props.entry.kind === "file" ? props.entry.file : undefined}>
          {(file) => (
            <small>
              +{file().additions} −{file().deletions}
            </small>
          )}
        </Show>
      </Button>
      <Show when={props.entry.kind === "directory" && !props.collapsed.has(props.entry.path)}>
        <For each={props.entry.kind === "directory" ? props.entry.children : []}>
          {(child) => (
            <DiffTreeEntry
              entry={child}
              depth={props.depth + 1}
              selectedPath={props.selectedPath}
              collapsed={props.collapsed}
              onToggle={props.onToggle}
              onFile={props.onFile}
            />
          )}
        </For>
      </Show>
    </>
  );
}

function TreeEntry(props: {
  entry: FsEntry;
  depth: number;
  expanded: Record<string, FsEntry[]>;
  filter: string;
  onDirectory: (entry: FsEntry) => void;
  onFile: (path: string) => void;
}) {
  const children = () => props.expanded[props.entry.path];
  return (
    <>
      <Button
        class="workspace-tree-entry"
        data-component="file-tree-row"
        data-tree-depth={props.depth}
        onClick={() =>
          props.entry.kind === "directory"
            ? props.onDirectory(props.entry)
            : props.onFile(props.entry.path)
        }
      >
        {props.entry.kind === "directory" ? (
          <ChevronDown size={12} classList={{ collapsed: !children() }} />
        ) : (
          <File size={12} />
        )}
        <span>{props.entry.name}</span>
      </Button>
      <Show when={children()}>
        {(items) => (
          <For
            each={items().filter((entry) => treeEntryMatches(entry, props.filter, props.expanded))}
          >
            {(entry) => (
              <TreeEntry
                entry={entry}
                depth={props.depth + 1}
                expanded={props.expanded}
                filter={props.filter}
                onDirectory={props.onDirectory}
                onFile={props.onFile}
              />
            )}
          </For>
        )}
      </Show>
    </>
  );
}

function treeEntryMatches(
  entry: FsEntry,
  filter: string,
  expanded: Record<string, FsEntry[]>,
): boolean {
  const normalized = filter.trim().toLowerCase();
  return (
    !normalized ||
    entry.path.toLowerCase().includes(normalized) ||
    (expanded[entry.path] ?? []).some((child) => treeEntryMatches(child, normalized, expanded))
  );
}
