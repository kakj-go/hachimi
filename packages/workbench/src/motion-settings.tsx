import { listen } from "@tauri-apps/api/event";
import {
  commandFailure,
  commands,
  type AvatarCatalogSnapshot,
  type InteractionMotionBinding,
  type InteractionRegion,
  type MotionCatalogEntry,
  type MotionCatalogSnapshot,
  type MotionCategory,
  type MotionImportInspection,
  type MotionPlaybackMode,
  type MotionRootMode,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Badge,
  Button,
  Dialog,
  NumberField,
  PageHeading,
  SearchField,
  SelectField,
  StatusBanner,
  Switch as Toggle,
  Tabs,
  TextField,
  Trash2,
  Upload,
} from "@hachimi/ui";
import {
  For,
  Match,
  Show,
  Switch,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import {
  INTERACTION_REGIONS,
  MOTION_CATEGORIES,
  interactionRegionLabel,
  motionCategoryLabel,
  motionDescription,
  motionName,
  motionPlaybackLabel,
  motionRootLabel,
} from "./motion-localization";
import { MotionPreviewCanvas } from "./motion-preview";

type MotionSettingsTab = "motions" | "interactions";
type MotionSourceFilter = "all" | "builtin" | "user";

export function MotionSettingsPage() {
  const i18n = useI18n();
  const text = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const [tab, setTab] = createSignal<MotionSettingsTab>("motions");
  const [catalog, setCatalog] = createSignal<MotionCatalogSnapshot>({
    entries: [],
    bindings: [],
    disabledMotionIds: [],
  });
  const [avatars, setAvatars] = createSignal<AvatarCatalogSnapshot>({
    currentId: null,
    entries: [],
  });
  const [selectedMotionId, setSelectedMotionId] = createSignal<string>();
  const [selectedRegion, setSelectedRegion] = createSignal<InteractionRegion>("head_top");
  const [previewMirror, setPreviewMirror] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [category, setCategory] = createSignal<MotionCategory | "all">("all");
  const [source, setSource] = createSignal<MotionSourceFilter>("all");
  const [interactionQuery, setInteractionQuery] = createSignal("");
  const [inspection, setInspection] = createSignal<MotionImportInspection>();
  const [editing, setEditing] = createSignal<MotionCatalogEntry>();
  const [pendingDelete, setPendingDelete] = createSignal<MotionCatalogEntry>();
  const [name, setName] = createSignal("");
  const [description, setDescription] = createSignal("");
  const [editCategory, setEditCategory] = createSignal<MotionCategory>("gesture");
  const [playbackMode, setPlaybackMode] = createSignal<MotionPlaybackMode>("once");
  const [rootMode, setRootMode] = createSignal<MotionRootMode>("in_place");
  const [importRegion, setImportRegion] = createSignal<InteractionRegion | "">("");
  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const unlisteners: Array<() => void> = [];

  const selectedMotion = createMemo(() =>
    catalog().entries.find((entry) => entry.id === selectedMotionId()),
  );
  const filteredEntries = createMemo(() => {
    const normalized = query().trim().toLocaleLowerCase();
    return catalog().entries.filter((entry) => {
      const localizedSearch = `${entry.name} ${entry.nameZh ?? ""} ${entry.description} ${
        entry.descriptionZh ?? ""
      } ${entry.tags.join(" ")}`.toLocaleLowerCase();
      return (
        (category() === "all" || entry.category === category()) &&
        (source() === "all" || entry.source === source()) &&
        (!normalized || localizedSearch.includes(normalized))
      );
    });
  });
  const interactionEntries = createMemo(() => {
    const normalized = interactionQuery().trim().toLocaleLowerCase();
    const filtered = catalog().entries.filter(
      (entry) =>
        !normalized ||
        `${entry.name} ${entry.nameZh ?? ""}`.toLocaleLowerCase().includes(normalized),
    );
    const current = catalog().entries.find((entry) => entry.id === selectedBinding()?.motionId);
    return current && !filtered.some((entry) => entry.id === current.id)
      ? [current, ...filtered]
      : filtered;
  });
  const selectedBinding = createMemo(() =>
    catalog().bindings.find((binding) => binding.region === selectedRegion()),
  );

  function chooseInitialMotion(snapshot: MotionCatalogSnapshot) {
    if (snapshot.entries.some((entry) => entry.id === selectedMotionId())) return;
    const preferred =
      snapshot.entries.find(
        (entry) =>
          entry.source === "builtin" &&
          entry.category === "idle" &&
          /standard waiting/i.test(entry.name),
      ) ?? snapshot.entries[0];
    setSelectedMotionId(preferred?.id);
  }

  onMount(async () => {
    try {
      const [nextCatalog, nextAvatars] = await Promise.all([
        commands.listMotionCatalog(),
        commands.listAvatarModels(),
      ]);
      setCatalog(nextCatalog);
      setAvatars(nextAvatars);
      chooseInitialMotion(nextCatalog);
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
    // eslint-disable-next-line solid/reactivity -- catalog events intentionally reconcile live selections.
    void listen<MotionCatalogSnapshot>("motion:catalog-changed", ({ payload }) => {
      setCatalog(payload);
      chooseInitialMotion(payload);
    }).then((unlisten) => unlisteners.push(unlisten));
    void listen<AvatarCatalogSnapshot>("avatar:catalog-changed", ({ payload }) => {
      setAvatars(payload);
    }).then((unlisten) => unlisteners.push(unlisten));
  });
  onCleanup(() => unlisteners.forEach((unlisten) => unlisten()));

  createEffect(() => {
    const entries = filteredEntries();
    if (tab() !== "motions" || entries.length === 0) return;
    if (!entries.some((entry) => entry.id === selectedMotionId())) {
      setSelectedMotionId(entries[0]?.id);
    }
  });

  async function replaceBinding(
    region: InteractionRegion,
    motionId: string | undefined,
    patch: Partial<InteractionMotionBinding> = {},
  ) {
    try {
      const snapshot = await commands.setInteractionMotionBinding({
        region,
        motionId: motionId ?? null,
        cooldownMs: patch.cooldownMs ?? null,
        mirrorBySide: patch.mirrorBySide ?? null,
      });
      setCatalog(snapshot);
      if (motionId) {
        setSelectedMotionId(motionId);
      } else if (selectedRegion() === region) {
        setSelectedMotionId(undefined);
      }
      if (selectedRegion() === region) {
        const binding = snapshot.bindings.find((value) => value.region === region);
        setPreviewMirror(Boolean(binding?.mirrorBySide && region.startsWith("left_")));
      }
      setNotice({ tone: "success", text: text("互动绑定已更新", "Interaction binding updated") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  async function clearMotionBindings(motionId: string) {
    try {
      const snapshot = await commands.clearMotionInteractionBindings({ motionId });
      setCatalog(snapshot);
      setNotice({
        tone: "success",
        text: text("已解除此动作的全部互动绑定", "All interaction bindings cleared"),
      });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  async function setMotionEnabled(id: string, enabled: boolean) {
    try {
      const snapshot = await commands.setMotionEnabled({ id, enabled });
      setCatalog(snapshot);
      setNotice({
        tone: "success",
        text: enabled
          ? text("动作已启用", "Motion enabled")
          : text("动作已禁用，现有绑定将保留", "Motion disabled; existing bindings retained"),
      });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  async function resetRegion(region: InteractionRegion) {
    try {
      const snapshot = await commands.resetMotionBinding({ region });
      setCatalog(snapshot);
      const binding = snapshot.bindings.find((value) => value.region === region);
      setSelectedMotionId(binding?.motionId);
      setNotice({ tone: "success", text: text("已恢复区域默认动作", "Region default restored") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  async function resetAllRegions() {
    try {
      const snapshot = await commands.resetMotionBindings();
      setCatalog(snapshot);
      const binding = snapshot.bindings.find((value) => value.region === selectedRegion());
      setSelectedMotionId(binding?.motionId);
      setNotice({ tone: "success", text: text("已恢复全部默认绑定", "All defaults restored") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  function resetEditor(entry?: MotionCatalogEntry) {
    setName(entry?.name ?? "");
    setDescription(entry?.description ?? "");
    setEditCategory(entry?.category ?? "gesture");
    setPlaybackMode(entry?.playbackMode ?? "once");
    setRootMode(entry?.rootMode ?? "in_place");
    setImportRegion("");
  }

  async function inspectImport() {
    setBusy(true);
    setNotice(undefined);
    try {
      const value = await commands.inspectMotionFile();
      if (!value) return;
      setInspection(value);
      resetEditor();
      setName(value.originalFileName.replace(/\.vrma$/i, ""));
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }

  async function closeEditor() {
    const token = inspection()?.token;
    setInspection(undefined);
    setEditing(undefined);
    if (token) await commands.cancelMotionImport(token).catch(() => undefined);
  }

  async function commitImport() {
    const value = inspection();
    if (!value?.token || !name().trim()) return;
    setBusy(true);
    try {
      const snapshot = await commands.commitMotionImport({
        token: value.token,
        name: name().trim(),
        description: description().trim(),
        category: editCategory(),
        playbackMode: playbackMode(),
        rootMode: rootMode(),
        interactionRegion: importRegion() || null,
      });
      setCatalog(snapshot);
      const imported = snapshot.entries
        .filter((entry) => entry.source === "user" && entry.sha256 === value.sha256)
        .at(-1);
      setSelectedMotionId(imported?.id);
      setInspection(undefined);
      setNotice({ tone: "success", text: text("动作已导入", "Motion imported") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }

  function openEditor(entry: MotionCatalogEntry) {
    setEditing(entry);
    resetEditor(entry);
  }

  async function saveMetadata() {
    const entry = editing();
    if (!entry || !name().trim()) return;
    setBusy(true);
    try {
      setCatalog(
        await commands.updateMotionMetadata({
          id: entry.id,
          name: name().trim(),
          description: description().trim(),
          category: editCategory(),
          playbackMode: playbackMode(),
          rootMode: rootMode(),
        }),
      );
      setEditing(undefined);
      setNotice({ tone: "success", text: text("动作信息已保存", "Motion details saved") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }

  async function deleteMotion() {
    const entry = pendingDelete();
    if (!entry) return;
    try {
      const snapshot = await commands.deleteUserMotion(entry.id);
      setCatalog(snapshot);
      setPendingDelete(undefined);
      chooseInitialMotion(snapshot);
      setNotice({
        tone: "success",
        text: text("动作已删除，相关区域已恢复默认", "Motion deleted; defaults restored"),
      });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  function selectInteractionRegion(region: InteractionRegion) {
    setSelectedRegion(region);
    const binding = catalog().bindings.find((value) => value.region === region);
    setSelectedMotionId(binding?.motionId);
    setPreviewMirror(Boolean(binding?.mirrorBySide && region.startsWith("left_")));
  }

  return (
    <div class="settings-page settings-page-demo motion-settings-page">
      <PageHeading
        class="settings-page-heading"
        title={text("交互", "Interactions")}
        description={text(
          "管理内置与用户 VRMA，并为桌宠各个互动区域设置一个确定动作。",
          "Manage built-in and user VRMA assets and assign one deterministic motion per interaction region.",
        )}
        badge={`${catalog().entries.length} VRMA`}
      />

      <Tabs
        value={tab()}
        onChange={(value) => setTab(value as MotionSettingsTab)}
        tabs={[
          { value: "motions", label: text("动作", "Motions"), content: <></> },
          {
            value: "interactions",
            label: text("互动绑定", "Interaction bindings"),
            content: <></>,
          },
        ]}
      />

      <Show when={notice()}>
        {(value) => <StatusBanner tone={value().tone}>{value().text}</StatusBanner>}
      </Show>

      <div class="motion-settings-layout">
        <div class="motion-settings-browser">
          <Switch>
            <Match when={tab() === "motions"}>
              <div class="motion-filter-bar">
                <div class="motion-filter-control search">
                  <SearchField
                    label={text("搜索动作", "Search motions")}
                    value={query()}
                    onInput={(event) => setQuery(event.currentTarget.value)}
                  />
                </div>
                <div class="motion-filter-control category">
                  <SelectField
                    label={text("分类", "Category")}
                    value={category()}
                    options={[
                      { value: "all", label: text("全部分类", "All categories") },
                      ...MOTION_CATEGORIES.map((value) => ({
                        value,
                        label: motionCategoryLabel(value, i18n.locale()),
                      })),
                    ]}
                    onChange={(value) => setCategory(value as MotionCategory | "all")}
                  />
                </div>
                <div class="motion-filter-control source">
                  <SelectField
                    label={text("来源", "Source")}
                    value={source()}
                    options={[
                      { value: "all", label: text("全部来源", "All sources") },
                      { value: "builtin", label: text("内置动作", "Built-in") },
                      { value: "user", label: text("用户动作", "User") },
                    ]}
                    onChange={(value) => setSource(value as MotionSourceFilter)}
                  />
                </div>
                <div class="motion-filter-control upload">
                  <Button variant="primary" disabled={busy()} onClick={() => void inspectImport()}>
                    <Upload size={14} /> {text("上传 VRMA", "Upload VRMA")}
                  </Button>
                </div>
              </div>
              <div class="motion-entry-list" role="listbox" aria-label={text("动作库", "Library")}>
                <For each={filteredEntries()}>
                  {(entry) => {
                    const disabled = () => catalog().disabledMotionIds.includes(entry.id);
                    const boundRegions = () =>
                      catalog()
                        .bindings.filter((binding) => binding.motionId === entry.id)
                        .map((binding) => binding.region);
                    return (
                      <article
                        class="motion-entry-card"
                        classList={{
                          selected: selectedMotionId() === entry.id,
                          disabled: disabled(),
                        }}
                        role="option"
                        aria-selected={selectedMotionId() === entry.id}
                        tabIndex={0}
                        onClick={() => setSelectedMotionId(entry.id)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            setSelectedMotionId(entry.id);
                          }
                        }}
                      >
                        <div class="motion-entry-heading">
                          <div>
                            <strong>{motionName(entry, i18n.locale())}</strong>
                            <p>{motionDescription(entry, i18n.locale())}</p>
                          </div>
                          <Badge tone={entry.protected ? "warning" : "info"}>
                            {entry.protected
                              ? text("内置锁定", "Built-in locked")
                              : text("用户动作", "User motion")}
                          </Badge>
                        </div>
                        <div class="motion-entry-badges">
                          <Badge>{motionCategoryLabel(entry.category, i18n.locale())}</Badge>
                          <Badge>{motionPlaybackLabel(entry.playbackMode, i18n.locale())}</Badge>
                          <Badge>{motionRootLabel(entry.rootMode, i18n.locale())}</Badge>
                          <Badge tone={entry.hasFingerMotion ? "success" : "neutral"}>
                            {entry.hasFingerMotion
                              ? text(
                                  `${entry.fingerBoneCount} 根手指轨道`,
                                  `${entry.fingerBoneCount} finger tracks`,
                                )
                              : text("无手指轨道", "No finger motion")}
                          </Badge>
                          <Show when={disabled()}>
                            <Badge tone="warning">{text("已禁用", "Disabled")}</Badge>
                          </Show>
                          <For each={boundRegions()}>
                            {(region) => (
                              <Badge tone="info">
                                {text("已绑定：", "Bound: ")}
                                {interactionRegionLabel(region, i18n.locale())}
                              </Badge>
                            )}
                          </For>
                        </div>
                        <div
                          class="motion-entry-actions"
                          onClick={(event) => event.stopPropagation()}
                        >
                          <div class="motion-entry-binding-select">
                            <SelectField
                              label={text("绑定到互动区域", "Bind to interaction")}
                              value={boundRegions()[0] ?? ""}
                              disabled={disabled()}
                              options={[
                                {
                                  value: "",
                                  label: text("无（解除全部绑定）", "None (clear all)"),
                                },
                                ...INTERACTION_REGIONS.map((region) => ({
                                  value: region,
                                  label: interactionRegionLabel(region, i18n.locale()),
                                })),
                              ]}
                              onChange={(value) => {
                                if (value) {
                                  void replaceBinding(value as InteractionRegion, entry.id);
                                } else {
                                  void clearMotionBindings(entry.id);
                                }
                              }}
                            />
                          </div>
                          <Show when={!entry.protected}>
                            <Button size="small" onClick={() => openEditor(entry)}>
                              {text("编辑", "Edit")}
                            </Button>
                            <Button
                              size="small"
                              variant="danger"
                              onClick={() => setPendingDelete(entry)}
                            >
                              <Trash2 size={14} /> {text("删除", "Delete")}
                            </Button>
                          </Show>
                          <div class="motion-entry-enabled">
                            <span>{text("启用动作", "Enable motion")}</span>
                            <Toggle
                              checked={!disabled()}
                              label={text("启用动作", "Enable motion")}
                              onChange={(enabled) => void setMotionEnabled(entry.id, enabled)}
                            />
                          </div>
                        </div>
                      </article>
                    );
                  }}
                </For>
              </div>
            </Match>

            <Match when={tab() === "interactions"}>
              <div class="interaction-settings-layout">
                <nav class="interaction-region-list" aria-label={text("互动区域", "Regions")}>
                  <For each={INTERACTION_REGIONS}>
                    {(region) => {
                      const binding = () =>
                        catalog().bindings.find((value) => value.region === region);
                      const entry = () =>
                        catalog().entries.find((value) => value.id === binding()?.motionId);
                      return (
                        <Button
                          type="button"
                          classList={{ selected: selectedRegion() === region }}
                          onClick={() => selectInteractionRegion(region)}
                        >
                          <strong>{interactionRegionLabel(region, i18n.locale())}</strong>
                          <small>
                            {entry()
                              ? `${motionName(entry()!, i18n.locale())}${
                                  catalog().disabledMotionIds.includes(entry()!.id)
                                    ? ` · ${text("动作已禁用", "Motion disabled")}`
                                    : ""
                                }`
                              : text("未绑定", "Unbound")}
                          </small>
                        </Button>
                      );
                    }}
                  </For>
                </nav>
                <section class="interaction-binding-editor">
                  <h2>{interactionRegionLabel(selectedRegion(), i18n.locale())}</h2>
                  <SearchField
                    label={text("搜索可用动作", "Search available motions")}
                    value={interactionQuery()}
                    onInput={(event) => setInteractionQuery(event.currentTarget.value)}
                  />
                  <SelectField
                    label={text("绑定动作", "Bound motion")}
                    value={selectedBinding()?.motionId ?? ""}
                    options={[
                      { value: "", label: text("无", "None") },
                      ...interactionEntries().map((entry) => {
                        const disabled = catalog().disabledMotionIds.includes(entry.id);
                        const current = selectedBinding()?.motionId === entry.id;
                        return {
                          value: entry.id,
                          label: disabled
                            ? `${motionName(entry, i18n.locale())} · ${text("已禁用", "Disabled")}`
                            : motionName(entry, i18n.locale()),
                          disabled: disabled && !current,
                        };
                      }),
                    ]}
                    onChange={(value) => void replaceBinding(selectedRegion(), value || undefined)}
                  />
                  <Show when={selectedBinding()}>
                    {(binding) => (
                      <>
                        <NumberField
                          label={text("冷却时间（毫秒）", "Cooldown (ms)")}
                          min={0}
                          max={60_000}
                          step={100}
                          value={binding().cooldownMs}
                          onInput={(event) =>
                            void replaceBinding(selectedRegion(), binding().motionId, {
                              cooldownMs: Math.max(event.currentTarget.valueAsNumber || 0, 0),
                            })
                          }
                        />
                        <div class="motion-toggle-field">
                          <div>
                            <strong>
                              {text("按左右区域镜像", "Mirror for left/right regions")}
                            </strong>
                            <p>
                              {text(
                                "绑定到左侧区域时自动翻转支持镜像的动作；不支持镜像的 VRMA 不受影响。",
                                "Automatically flips mirrorable motions for left-side regions; unsupported VRMA files are unchanged.",
                              )}
                            </p>
                          </div>
                          <Toggle
                            checked={binding().mirrorBySide}
                            label={text("按左右区域镜像", "Mirror for left/right regions")}
                            onChange={(mirrorBySide) =>
                              void replaceBinding(selectedRegion(), binding().motionId, {
                                mirrorBySide,
                              })
                            }
                          />
                        </div>
                      </>
                    )}
                  </Show>
                  <div class="settings-card-actions">
                    <Button onClick={() => void resetRegion(selectedRegion())}>
                      {text("恢复此区域默认", "Restore region default")}
                    </Button>
                    <Button onClick={() => void resetAllRegions()}>
                      {text("恢复全部默认", "Restore all defaults")}
                    </Button>
                  </div>
                </section>
              </div>
            </Match>
          </Switch>
        </div>

        <aside class="motion-settings-preview">
          <Show
            when={avatars().entries.length > 0}
            fallback={<StatusBanner>{text("没有可用预览模型", "No preview avatar")}</StatusBanner>}
          >
            <MotionPreviewCanvas
              avatars={avatars()}
              entries={catalog().entries}
              motionId={selectedMotionId()}
              mirror={previewMirror()}
              onMirrorChange={setPreviewMirror}
            />
          </Show>
          <Show when={selectedMotion()}>
            {(entry) => (
              <div class="motion-preview-summary">
                <strong>{motionName(entry(), i18n.locale())}</strong>
                <p>{motionDescription(entry(), i18n.locale())}</p>
              </div>
            )}
          </Show>
        </aside>
      </div>

      <MotionEditorDialog
        open={Boolean(inspection()) || Boolean(editing())}
        importing={Boolean(inspection())}
        inspection={inspection()}
        name={name()}
        description={description()}
        category={editCategory()}
        playbackMode={playbackMode()}
        rootMode={rootMode()}
        interactionRegion={importRegion()}
        busy={busy()}
        onName={setName}
        onDescription={setDescription}
        onCategory={setEditCategory}
        onPlaybackMode={setPlaybackMode}
        onRootMode={setRootMode}
        onInteractionRegion={setImportRegion}
        onCancel={() => void closeEditor()}
        onConfirm={() => (inspection() ? void commitImport() : void saveMetadata())}
      />

      <Dialog
        open={Boolean(pendingDelete())}
        title={text("删除用户动作", "Delete user motion")}
        description={text(
          "删除后，使用此动作的互动区域会恢复对应的内置默认动作。",
          "Regions using this motion will be restored to their built-in defaults.",
        )}
        onOpenChange={(open) => !open && setPendingDelete(undefined)}
      >
        <div class="dialog-actions">
          <Button variant="ghost" onClick={() => setPendingDelete(undefined)}>
            {text("取消", "Cancel")}
          </Button>
          <Button variant="danger" onClick={() => void deleteMotion()}>
            {text("删除", "Delete")}
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

function MotionEditorDialog(props: {
  open: boolean;
  importing: boolean;
  inspection: MotionImportInspection | undefined;
  name: string;
  description: string;
  category: MotionCategory;
  playbackMode: MotionPlaybackMode;
  rootMode: MotionRootMode;
  interactionRegion: InteractionRegion | "";
  busy: boolean;
  onName: (value: string) => void;
  onDescription: (value: string) => void;
  onCategory: (value: MotionCategory) => void;
  onPlaybackMode: (value: MotionPlaybackMode) => void;
  onRootMode: (value: MotionRootMode) => void;
  onInteractionRegion: (value: InteractionRegion | "") => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const i18n = useI18n();
  const text = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  return (
    <Dialog
      open={props.open}
      title={
        props.importing
          ? text("上传用户动作", "Upload user motion")
          : text("编辑用户动作", "Edit user motion")
      }
      onOpenChange={(open) => !open && props.onCancel()}
    >
      <Show when={props.inspection}>
        {(value) => (
          <StatusBanner tone={value().warnings.length > 0 ? "warning" : "success"}>
            {`${value().originalFileName} · ${(value().durationMs / 1_000).toFixed(2)}s · ${
              value().animatedBones.length
            } bones · ${value().fingerBoneCount} fingers`}
          </StatusBanner>
        )}
      </Show>
      <TextField
        label={text("名称", "Name")}
        value={props.name}
        onInput={(event) => props.onName(event.currentTarget.value)}
      />
      <TextField
        label={text("描述", "Description")}
        value={props.description}
        onInput={(event) => props.onDescription(event.currentTarget.value)}
      />
      <SelectField
        label={text("分类", "Category")}
        value={props.category}
        options={MOTION_CATEGORIES.map((value) => ({
          value,
          label: motionCategoryLabel(value, i18n.locale()),
        }))}
        onChange={(value) => props.onCategory(value as MotionCategory)}
      />
      <SelectField
        label={text("播放方式", "Playback")}
        value={props.playbackMode}
        options={(["once", "loop", "hold"] as const).map((value) => ({
          value,
          label: motionPlaybackLabel(value, i18n.locale()),
        }))}
        onChange={(value) => props.onPlaybackMode(value as MotionPlaybackMode)}
      />
      <SelectField
        label={text("Root 模式", "Root mode")}
        value={props.rootMode}
        options={(["discard", "in_place", "stage"] as const).map((value) => ({
          value,
          label: motionRootLabel(value, i18n.locale()),
        }))}
        onChange={(value) => props.onRootMode(value as MotionRootMode)}
      />
      <Show when={props.importing}>
        <SelectField
          label={text("绑定互动区域（可选）", "Interaction region (optional)")}
          value={props.interactionRegion}
          options={[
            { value: "", label: text("暂不绑定", "Do not bind") },
            ...INTERACTION_REGIONS.map((value) => ({
              value,
              label: interactionRegionLabel(value, i18n.locale()),
            })),
          ]}
          onChange={(value) => props.onInteractionRegion(value as InteractionRegion | "")}
        />
      </Show>
      <div class="dialog-actions">
        <Button variant="ghost" onClick={props.onCancel}>
          {text("取消", "Cancel")}
        </Button>
        <Button
          variant="primary"
          disabled={props.busy || !props.name.trim()}
          onClick={props.onConfirm}
        >
          {text("保存", "Save")}
        </Button>
      </div>
    </Dialog>
  );
}
