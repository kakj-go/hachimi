import {
  commandFailure,
  type DiffReadFileResponse,
  type DiffScope,
  type FsEntry,
  type FsFileChunk,
  type FsSearchSnapshot,
  type FsWatchRegistration,
  type RunDiffSnapshot,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  AlertTriangle,
  Button,
  ChevronDown,
  File,
  FolderOpen,
  GitBranch,
  GitFork,
  RefreshCw,
  Search,
  SearchField,
  X,
} from "@hachimi/ui";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, untrack } from "solid-js";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { runMutationContext } from "./mutation-context";
import { WorkspaceFileEditor } from "./workspace-file-editor";
import { WorkspaceGitPanel } from "./workspace-git-panel";

const CHUNK_BYTES = 256 * 1024;
const MAX_EDIT_BYTES = 2 * 1024 * 1024;

export function WorkspaceBrowser(props: {
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  gitRemoteMutationsEnabled?: boolean;
}) {
  const i18n = useI18n();
  const [tab, setTab] = createSignal<"files" | "search" | "diff" | "git">("files");
  const [entries, setEntries] = createSignal<FsEntry[]>([]);
  const [expanded, setExpanded] = createSignal<Record<string, FsEntry[]>>({});
  const [selectedPath, setSelectedPath] = createSignal<string>();
  const [chunks, setChunks] = createSignal<FsFileChunk[]>([]);
  const [savedText, setSavedText] = createSignal("");
  const [draft, setDraft] = createSignal("");
  const [fileEtag, setFileEtag] = createSignal<string>();
  const [saveConflict, setSaveConflict] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [searchSnapshot, setSearchSnapshot] = createSignal<FsSearchSnapshot>();
  const [diffScope, setDiffScope] = createSignal<"run" | "checkout">("run");
  const [diff, setDiff] = createSignal<RunDiffSnapshot>();
  const [diffFileChunks, setDiffFileChunks] = createSignal<Record<string, DiffReadFileResponse[]>>(
    {},
  );
  const [watchInvalidated, setWatchInvalidated] = createSignal(false);
  const [gitRevision, setGitRevision] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  let watch: FsWatchRegistration | undefined;
  let searchRequest = 0;
  let diffRequest = 0;
  let fileRequest = 0;
  let searchTimer: number | undefined;

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
    const run = latestRun();
    if (diffScope() === "run" && run) return { kind: "run", run_id: run.id };
    const currentCheckoutId = checkoutId();
    return currentCheckoutId ? { kind: "checkout", checkout_id: currentCheckoutId } : undefined;
  };
  const chunksForDiff = (path: string) => diffFileChunks()[path] ?? [];
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
      }
    } catch (error) {
      if (request === diffRequest) setFailure(commandFailure(error).message);
    } finally {
      if (request === diffRequest) setBusy(false);
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
    setDiffFileChunks({});
    setWatchInvalidated(false);
    void loadDirectory();
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
          setGitRevision((revision) => revision + 1);
          void loadDirectory();
          if (untrack(tab) === "diff") void loadDiff();
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
    const commandPort = props.commandPort;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void commandPort
      .onWorkspaceSearch((snapshot) => {
        if (disposed) return;
        const active = untrack(searchSnapshot);
        if (
          active?.searchId === snapshot.searchId &&
          snapshot.generation >= active.generation &&
          snapshot.query === untrack(query).trim()
        ) {
          setSearchSnapshot(snapshot);
        }
      })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch((error) => setFailure(commandFailure(error).message));
    onCleanup(() => {
      disposed = true;
      unlisten?.();
    });
  });

  createEffect(() => {
    const value = query().trim();
    const commandPort = props.commandPort;
    if (searchTimer) window.clearTimeout(searchTimer);
    const request = ++searchRequest;
    if (!value) {
      const active = untrack(searchSnapshot);
      if (active) void commandPort.cancelWorkspaceFileSearch(active.searchId).catch(() => false);
      setSearchSnapshot(undefined);
      return;
    }
    // eslint-disable-next-line solid/reactivity -- debounced search intentionally reads the latest snapshot.
    searchTimer = window.setTimeout(() => {
      const active = untrack(searchSnapshot);
      const operation = active
        ? commandPort.updateWorkspaceFileSearch({
            searchId: active.searchId,
            expectedGeneration: active.generation,
            query: value,
          })
        : commandPort.startWorkspaceFileSearch({ ...context(), query: value, maxResults: 200 });
      void operation
        .then((snapshot) => {
          if (request === searchRequest && untrack(query).trim() === snapshot.query) {
            setSearchSnapshot(snapshot);
          } else {
            void commandPort.cancelWorkspaceFileSearch(snapshot.searchId).catch(() => false);
          }
        })
        .catch((error) => {
          if (request === searchRequest) setFailure(commandFailure(error).message);
        });
    }, 150);
    onCleanup(() => {
      if (searchTimer) window.clearTimeout(searchTimer);
    });
  });

  createEffect(() => {
    diffScope();
    if (tab() === "diff") void loadDiff();
  });

  onCleanup(() => {
    const active = searchSnapshot();
    if (active)
      void props.commandPort.cancelWorkspaceFileSearch(active.searchId).catch(() => false);
  });

  return (
    <aside
      class="workspace-browser"
      data-component="workspace"
      aria-label={i18n.locale() === "zh-CN" ? "工作区" : "Workspace"}
    >
      <header class="workspace-browser-tabs">
        <Button
          data-testid="workspace-files-tab"
          classList={{ active: tab() === "files" }}
          onClick={() => setTab("files")}
        >
          <FolderOpen size={14} /> {i18n.locale() === "zh-CN" ? "文件" : "Files"}
        </Button>
        <Button classList={{ active: tab() === "search" }} onClick={() => setTab("search")}>
          <Search size={14} /> {i18n.locale() === "zh-CN" ? "搜索" : "Search"}
        </Button>
        <Button
          data-testid="workspace-diff-tab"
          classList={{ active: tab() === "diff" }}
          onClick={() => setTab("diff")}
        >
          <GitBranch size={14} /> Diff
        </Button>
        <Button
          data-testid="workspace-git-tab"
          classList={{ active: tab() === "git" }}
          onClick={() => setTab("git")}
        >
          <GitFork size={14} /> Git
        </Button>
        <Button
          class="workspace-refresh"
          aria-label={i18n.locale() === "zh-CN" ? "刷新文件" : "Refresh files"}
          title={i18n.locale() === "zh-CN" ? "刷新文件" : "Refresh files"}
          disabled={busy()}
          onClick={() => void loadDirectory("", false, null, true)}
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
      <Show when={tab() === "files"}>
        <div class="workspace-file-tree" data-component="file-tree">
          <For each={entries()}>
            {(entry) => (
              <TreeEntry
                entry={entry}
                depth={0}
                expanded={expanded()}
                onDirectory={toggleDirectory}
                onFile={(path) => void openFile(path)}
              />
            )}
          </For>
        </div>
      </Show>
      <Show when={tab() === "search"}>
        <div class="workspace-search-panel">
          <SearchField
            label={i18n.locale() === "zh-CN" ? "搜索文件路径" : "Search file paths"}
            autofocus
            value={query()}
            placeholder={i18n.locale() === "zh-CN" ? "搜索文件路径" : "Search file paths"}
            onInput={(event) => setQuery(event.currentTarget.value)}
          />
          <For each={searchSnapshot()?.results ?? []}>
            {(result) => (
              <Button
                onClick={() => {
                  setTab("files");
                  void openFile(result.path);
                }}
              >
                <File size={13} />
                <span>{highlightPath(result.path, result.matchIndices)}</span>
              </Button>
            )}
          </For>
        </div>
      </Show>
      <Show when={tab() === "diff"}>
        <div class="workspace-diff-panel" data-component="diff">
          <div class="workspace-diff-scope">
            <Button
              classList={{ active: diffScope() === "run" }}
              onClick={() => setDiffScope("run")}
            >
              {i18n.locale() === "zh-CN" ? "当前运行" : "Run"}
            </Button>
            <Button
              classList={{ active: diffScope() === "checkout" }}
              onClick={() => setDiffScope("checkout")}
            >
              Checkout
            </Button>
          </div>
          <For each={diff()?.files ?? []}>
            {(file) => (
              <details class="workspace-diff-file">
                <summary>
                  <span>{file.status}</span>
                  <strong>{file.path}</strong>
                  <small>
                    +{file.additions} −{file.deletions}
                  </small>
                </summary>
                <Show
                  when={!file.binary && !file.tooLarge}
                  fallback={
                    <Show when={!file.binary} fallback={<p>Binary file</p>}>
                      <div class="workspace-diff-artifact">
                        <Show
                          when={chunksForDiff(file.path).length > 0}
                          fallback={
                            <Button disabled={busy()} onClick={() => void loadDiffFile(file.path)}>
                              {i18n.locale() === "zh-CN" ? "加载完整 Diff" : "Load full Diff"}
                            </Button>
                          }
                        >
                          <pre>
                            {chunksForDiff(file.path)
                              .map((chunk) => chunk.utf8Text ?? "")
                              .join("")}
                          </pre>
                          <Show when={lastDiffChunk(file.path)?.eof === false}>
                            <Button
                              disabled={busy()}
                              onClick={() => {
                                const last = lastDiffChunk(file.path);
                                if (last) void loadDiffFile(file.path, last.nextOffset, last.etag);
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
                  <For each={file.hunks}>
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
              </details>
            )}
          </For>
        </div>
      </Show>
      <Show when={tab() === "git"}>
        <WorkspaceGitPanel
          snapshot={props.snapshot}
          commandPort={props.commandPort}
          revision={gitRevision()}
          gitRemoteMutationsEnabled={props.gitRemoteMutationsEnabled !== false}
        />
      </Show>
      <Show when={selectedPath()}>
        {(path) => (
          <>
            <WorkspaceFileEditor
              path={path()}
              value={draft()}
              editable={fileEditable()}
              dirty={dirty()}
              saving={saving()}
              conflict={saveConflict()}
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
              onClose={() => {
                fileRequest += 1;
                setSelectedPath(undefined);
                setChunks([]);
                setSavedText("");
                setDraft("");
                setFileEtag(undefined);
                setSaveConflict(false);
              }}
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
    </aside>
  );
}

function TreeEntry(props: {
  entry: FsEntry;
  depth: number;
  expanded: Record<string, FsEntry[]>;
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
          <For each={items()}>
            {(entry) => (
              <TreeEntry
                entry={entry}
                depth={props.depth + 1}
                expanded={props.expanded}
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

function highlightPath(path: string, indices: number[]) {
  const selected = new Set(indices);
  const tokens = Array.from(path, (character, index) => ({
    character,
    marked: selected.has(index),
  }));
  return (
    <For each={tokens}>
      {(token) => (token.marked ? <mark>{token.character}</mark> : token.character)}
    </For>
  );
}
