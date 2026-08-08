import { listen } from "@tauri-apps/api/event";
import {
  commandFailure,
  commands,
  type AvatarAssessment,
  type AvatarCatalogSnapshot,
  type AvatarCapability,
  type AvatarEntry,
  type AvatarFormat,
  type AvatarImportInspection,
  type SpeechRecognitionRuntimeState,
  type VoiceCatalogSnapshot,
  type VoiceComputeMode,
  type VoiceModelEntry,
  type VoiceModelInspection,
  type VoiceRuntimeState,
} from "@hachimi/contracts";
import { useI18n, type AppLocale } from "@hachimi/i18n";
import {
  Badge,
  Button,
  Dialog,
  NumberField,
  PageHeading,
  Play,
  RangeField,
  ResourceCard,
  ResourceList,
  SelectField,
  SettingsCard,
  SettingsRow,
  SettingsSection,
  Square,
  StatusBanner,
  Switch as Toggle,
  Trash2,
  TextField,
} from "@hachimi/ui";
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { AvatarCardPreview } from "./avatar-card-preview";

type CatalogEntry = AvatarEntry;
type CatalogSnapshot = AvatarCatalogSnapshot;

export function ResourceSettingsPage() {
  const i18n = useI18n();
  const [snapshot, setSnapshot] = createSignal<CatalogSnapshot>({ entries: [], currentId: null });
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const [pendingDelete, setPendingDelete] = createSignal<CatalogEntry>();
  const [avatarInspection, setAvatarInspection] = createSignal<AvatarImportInspection>();
  const isAvatar = () => true;
  const entries = () => snapshot().entries as CatalogEntry[];
  async function load() {
    setSnapshot(await commands.listAvatarModels());
  }
  async function importResource() {
    if (!name().trim()) {
      setNotice({ tone: "danger", text: i18n.t("settings.resource.invalidName") });
      return;
    }
    setBusy(true);
    setNotice(undefined);
    try {
      const inspection = await commands.inspectAvatarModel();
      if (inspection) {
        setAvatarInspection(inspection);
      } else {
        setNotice({ tone: "success", text: i18n.t("settings.resource.cancelled") });
      }
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }
  async function closeAvatarInspection() {
    const token = avatarInspection()?.token;
    setAvatarInspection(undefined);
    if (token) await commands.cancelAvatarModelImport(token).catch(() => undefined);
  }
  async function confirmAvatarImport() {
    const inspection = avatarInspection();
    if (!inspection?.token) return;
    setBusy(true);
    try {
      const next = await commands.commitAvatarModelImport({
        token: inspection.token,
        name: name().trim(),
      });
      setSnapshot(next);
      setAvatarInspection(undefined);
      setName("");
      setNotice({ tone: "success", text: i18n.t("settings.resource.imported") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }
  async function selectResource(id: string) {
    try {
      setSnapshot(await commands.selectAvatarModel(id));
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }
  async function deleteResource(entry: CatalogEntry) {
    setPendingDelete(entry);
  }
  async function confirmDelete() {
    const entry = pendingDelete();
    if (!entry) return;
    try {
      setSnapshot(await commands.deleteAvatarModel(entry.id));
      setPendingDelete(undefined);
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }
  onMount(() => {
    void load().catch((error) =>
      setNotice({ tone: "danger", text: commandFailure(error).message }),
    );
  });
  return (
    <div class="settings-page settings-page-demo">
      <PageHeading
        class="settings-page-heading"
        title={i18n.t("settings.avatar")}
        description={i18n.t("settings.avatar.description")}
        badge="VRM 0.x / 1.0 · Runtime Ready · ≤ 200MB"
      />
      <Show when={isAvatar()}>
        <SettingsSection title={i18n.t("settings.avatar.importTitle")}>
          <StatusBanner>{i18n.t("settings.avatar.sketchfabHint")}</StatusBanner>
          <SettingsCard class="settings-card unified-settings-card resource-import-card">
            <SettingsRow
              label={i18n.t("settings.resourceName")}
              description={i18n.t("settings.resource.sharedBlob")}
            >
              <TextField
                label={i18n.t("settings.resourceName")}
                value={name()}
                placeholder={i18n.t("settings.avatar.nameExample")}
                onInput={(event) => setName(event.currentTarget.value)}
              />
            </SettingsRow>
            <div class="settings-card-actions">
              <Button variant="primary" disabled={busy()} onClick={() => void importResource()}>
                {i18n.t("settings.avatar.import")}
              </Button>
            </div>
          </SettingsCard>
        </SettingsSection>
      </Show>
      <Show when={notice()}>
        {(value) => <StatusBanner tone={value().tone}>{value().text}</StatusBanner>}
      </Show>
      <Show when={isAvatar()}>
        <SettingsSection title={`${i18n.t("settings.catalog")} · ${entries().length}`}>
          <Show
            when={entries().length > 0}
            fallback={<div class="empty-resource">{i18n.t("common.noResources")}</div>}
          >
            <ResourceList label={i18n.t("settings.avatar")}>
              <For each={entries()}>
                {(entry) => (
                  <ResourceCard
                    title={entry.name}
                    subtitle={`${entry.originalFileName} · ${formatBytes(entry.sizeBytes)}`}
                    media={<AvatarCardPreview entryId={entry.id} name={entry.name} />}
                    current={entry.isCurrent}
                    tone={
                      resolvedAvatarAssessment(entry).compatibility !== "runtime_ready"
                        ? "danger"
                        : "default"
                    }
                    meta={
                      <div class="avatar-resource-meta">
                        <div class="avatar-badge-row">
                          <Badge>{avatarFormatLabel(entry.format ?? "glb")}</Badge>
                          <Badge
                            tone={avatarCompatibilityTone(
                              resolvedAvatarAssessment(entry).compatibility,
                            )}
                          >
                            {avatarCompatibilityLabel(
                              resolvedAvatarAssessment(entry).compatibility,
                              i18n.locale(),
                            )}
                          </Badge>
                          <Show when={resolvedAvatarAssessment(entry).issues.length > 0}>
                            <Badge tone="warning">
                              {i18n
                                .t("settings.avatar.warningSummary")
                                .replace(
                                  "{count}",
                                  String(resolvedAvatarAssessment(entry).issues.length),
                                )}
                            </Badge>
                          </Show>
                        </div>
                        <span>{`${entry.sha256.slice(0, 16)}… · ${formatDate(entry.importedAt)}`}</span>
                      </div>
                    }
                    details={
                      <details class="avatar-assessment-details">
                        <summary>{i18n.t("settings.avatar.assessmentDetails")}</summary>
                        <AvatarAssessmentDetails assessment={resolvedAvatarAssessment(entry)} />
                      </details>
                    }
                    actions={
                      <>
                        <Show
                          when={entry.isCurrent}
                          fallback={
                            <Button
                              size="small"
                              disabled={
                                resolvedAvatarAssessment(entry).compatibility !== "runtime_ready"
                              }
                              onClick={() => void selectResource(entry.id)}
                            >
                              {i18n.t("common.select")}
                            </Button>
                          }
                        >
                          <Badge tone="success">{i18n.t("common.current")}</Badge>
                        </Show>
                        <Show when={!entry.protected}>
                          <Button
                            size="small"
                            variant="danger"
                            onClick={() => void deleteResource(entry)}
                          >
                            <Trash2 size={14} /> {i18n.t("common.delete")}
                          </Button>
                        </Show>
                      </>
                    }
                  />
                )}
              </For>
            </ResourceList>
          </Show>
        </SettingsSection>
        <Dialog
          open={Boolean(avatarInspection())}
          title={i18n.t("settings.avatar.inspectionTitle")}
          description={i18n.t("settings.avatar.inspectionDescription")}
          onOpenChange={(open) => {
            if (!open) void closeAvatarInspection();
          }}
        >
          <Show when={avatarInspection()}>
            {(inspection) => (
              <div class="avatar-inspection-dialog">
                <div class="avatar-inspection-heading">
                  <div>
                    <strong>{inspection().originalFileName}</strong>
                    <span>{formatBytes(inspection().sizeBytes)}</span>
                  </div>
                  <div class="avatar-badge-row">
                    <Badge>{avatarFormatLabel(inspection().format)}</Badge>
                    <Badge tone={avatarCompatibilityTone(inspection().assessment.compatibility)}>
                      {avatarCompatibilityLabel(
                        inspection().assessment.compatibility,
                        i18n.locale(),
                      )}
                    </Badge>
                  </div>
                </div>
                <AvatarAssessmentDetails assessment={inspection().assessment} />
                <div class="dialog-actions">
                  <Button variant="ghost" onClick={() => void closeAvatarInspection()}>
                    {i18n.t("common.cancel")}
                  </Button>
                  <Button
                    variant="primary"
                    disabled={!inspection().token || busy()}
                    onClick={() => void confirmAvatarImport()}
                  >
                    {inspection().token
                      ? i18n.t("settings.avatar.confirmImport")
                      : i18n.t("settings.avatar.importBlocked")}
                  </Button>
                </div>
              </div>
            )}
          </Show>
        </Dialog>
        <Dialog
          open={Boolean(pendingDelete())}
          title={i18n.t("settings.resource.deleteTitle")}
          description={i18n
            .t("settings.resource.confirmDelete")
            .replace("{name}", pendingDelete()?.name ?? "")}
          onOpenChange={(open) => {
            if (!open) setPendingDelete(undefined);
          }}
        >
          <div class="dialog-actions">
            <Button variant="ghost" onClick={() => setPendingDelete(undefined)}>
              {i18n.t("common.cancel")}
            </Button>
            <Button variant="danger" onClick={() => void confirmDelete()}>
              {i18n.t("common.delete")}
            </Button>
          </div>
        </Dialog>
      </Show>
    </div>
  );
}

export function VoiceSettingsPage() {
  const i18n = useI18n();
  const [catalog, setCatalog] = createSignal<VoiceCatalogSnapshot>({
    entries: [],
    currentId: "builtin-melo-zh-en",
  });
  const [runtime, setRuntime] = createSignal<VoiceRuntimeState>();
  const [recognition, setRecognition] = createSignal<SpeechRecognitionRuntimeState>();
  const [inspection, setInspection] = createSignal<VoiceModelInspection>();
  const [pendingDelete, setPendingDelete] = createSignal<VoiceModelEntry>();
  const [name, setName] = createSignal("");
  const [licenseAcknowledged, setLicenseAcknowledged] = createSignal(false);
  const [speakerId, setSpeakerId] = createSignal(0);
  const [speed, setSpeed] = createSignal(100);
  const [computeMode, setComputeMode] = createSignal<VoiceComputeMode>("auto");
  const [recognitionComputeMode, setRecognitionComputeMode] =
    createSignal<VoiceComputeMode>("auto");
  const [busy, setBusy] = createSignal(false);
  const [recognitionBusy, setRecognitionBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const unlisteners: Array<() => void> = [];

  async function load() {
    const [nextCatalog, nextRuntime, nextRecognition] = await Promise.all([
      commands.listVoiceModels(),
      commands.getVoiceRuntimeState(),
      commands.getSpeechRecognitionState(),
    ]);
    setCatalog(nextCatalog);
    setRuntime(nextRuntime);
    setSpeed(nextRuntime.speedPercent);
    setComputeMode(nextRuntime.computeMode);
    setRecognition(nextRecognition);
    setRecognitionComputeMode(nextRecognition.computeMode);
  }

  async function updateRecognitionSettings(nextComputeMode: VoiceComputeMode) {
    setRecognitionBusy(true);
    setNotice(undefined);
    try {
      const next = await commands.updateSpeechRecognitionSettings({
        computeMode: nextComputeMode,
      });
      setRecognition(next);
      setRecognitionComputeMode(next.computeMode);
      setNotice({ tone: "success", text: i18n.t("settings.voice.inputBackendSaved") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
      await commands
        .getSpeechRecognitionState()
        .then((value) => {
          setRecognition(value);
          setRecognitionComputeMode(value.computeMode);
        })
        .catch(() => undefined);
    } finally {
      setRecognitionBusy(false);
    }
  }

  async function updateVoiceSettings(speedPercent: number, nextComputeMode: VoiceComputeMode) {
    setBusy(true);
    try {
      const next = await commands.updateVoiceSettings({
        speedPercent,
        computeMode: nextComputeMode,
      });
      setRuntime(next);
      setSpeed(next.speedPercent);
      setComputeMode(next.computeMode);
      setNotice({ tone: "success", text: i18n.t("settings.voice.profileSaved") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
      const previous = runtime();
      if (previous) {
        setSpeed(previous.speedPercent);
        setComputeMode(previous.computeMode);
      }
    } finally {
      setBusy(false);
    }
  }

  async function inspectModel() {
    if (!name().trim()) {
      setNotice({ tone: "danger", text: i18n.t("settings.resource.invalidName") });
      return;
    }
    setBusy(true);
    setNotice(undefined);
    try {
      const next = await commands.inspectVoiceModel();
      if (next) {
        setInspection(next);
        setLicenseAcknowledged(false);
        setSpeakerId(next.suggestedSpeakerId);
      } else {
        setNotice({ tone: "success", text: i18n.t("settings.resource.cancelled") });
      }
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }

  async function closeInspection() {
    const token = inspection()?.token;
    setInspection(undefined);
    setSpeakerId(0);
    if (token) await commands.cancelVoiceModelImport(token).catch(() => undefined);
  }

  async function commitImport() {
    const current = inspection();
    if (!current?.token) return;
    setBusy(true);
    try {
      const next = await commands.commitVoiceModelImport({
        token: current.token,
        name: name().trim(),
        licenseAcknowledged: licenseAcknowledged(),
        speakerId: speakerId(),
      });
      setCatalog(next);
      setInspection(undefined);
      setName("");
      setLicenseAcknowledged(false);
      setSpeakerId(0);
      setNotice({ tone: "success", text: i18n.t("settings.resource.imported") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }

  async function selectModel(id: string) {
    setBusy(true);
    try {
      setCatalog(await commands.selectVoiceModel(id));
      setRuntime(await commands.getVoiceRuntimeState());
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }

  async function confirmDelete() {
    const entry = pendingDelete();
    if (!entry) return;
    try {
      setCatalog(await commands.deleteVoiceModel(entry.id));
      setPendingDelete(undefined);
      setRuntime(await commands.getVoiceRuntimeState());
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  async function setMuted(muted: boolean) {
    try {
      setRuntime(await commands.setMuted(muted));
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  async function preview() {
    try {
      setRuntime(await commands.previewDefaultVoice());
      setNotice({ tone: "success", text: i18n.t("settings.voice.previewStarted") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  onMount(() => {
    void load().catch((error) =>
      setNotice({ tone: "danger", text: commandFailure(error).message }),
    );
    void Promise.all([
      listen<VoiceCatalogSnapshot>("voice:catalog-changed", ({ payload }) => setCatalog(payload)),
      listen<VoiceRuntimeState>("voice-runtime-changed", ({ payload }) => {
        setRuntime(payload);
        setSpeed(payload.speedPercent);
        setComputeMode(payload.computeMode);
      }),
      listen<SpeechRecognitionRuntimeState>("speech-recognition-state-changed", ({ payload }) => {
        setRecognition(payload);
        setRecognitionComputeMode(payload.computeMode);
      }),
    ]).then((values) => unlisteners.push(...values));
  });
  onCleanup(() => unlisteners.forEach((unlisten) => unlisten()));

  return (
    <div class="settings-page settings-page-demo">
      <PageHeading
        class="settings-page-heading"
        title={i18n.t("settings.voice")}
        description={i18n.t("settings.voice.description")}
        badge="sherpa-onnx 1.13.4"
      />

      <SettingsSection title={i18n.t("settings.voice.inputTitle")}>
        <StatusBanner>{i18n.t("settings.voice.inputDescription")}</StatusBanner>
        <SettingsCard class="settings-card voice-runtime-card">
          <SettingsRow
            label={recognition()?.modelName ?? "SenseVoice-Small INT8"}
            description={i18n.t("settings.voice.recognitionDescription")}
          >
            <Badge tone={recognition()?.installed ? "success" : "danger"}>
              {recognition()?.installed
                ? i18n.t("settings.voice.inputBundledState")
                : i18n.t("settings.voice.unavailable")}
            </Badge>
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.inputRuntime")}
            description={recognition()?.provider ?? "sherpa-onnx 1.13.4"}
          >
            <SelectField
              label={i18n.t("settings.voice.computeMode")}
              value={recognitionComputeMode()}
              disabled={recognitionBusy() || Boolean(recognition()?.loading)}
              options={[
                { value: "auto", label: i18n.t("settings.voice.computeAuto") },
                { value: "direct_ml", label: "DirectML" },
                { value: "cpu", label: "CPU" },
              ]}
              onChange={(value) => void updateRecognitionSettings(value as VoiceComputeMode)}
            />
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.backend")}
            description={
              recognition()?.computeDevice
                ? `${recognition()!.computeDevice!.name} · Adapter ${recognition()!.computeDevice!.deviceId} · ${recognition()!.computeDevice!.dedicatedMemoryMb} MB`
                : (recognition()?.fallbackReason ?? i18n.t("settings.voice.backendDescription"))
            }
          >
            <Show when={recognition()?.backend}>
              <Badge tone={recognition()?.fallbackReason ? "warning" : "info"}>
                {recognition()?.backend === "direct_ml" ? "DirectML" : "CPU"}
              </Badge>
            </Show>
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.inputLanguages")}
            description={(
              recognition()?.languages ?? ["zh-CN", "en-US", "ja-JP", "ko-KR", "yue"]
            ).join(" / ")}
          >
            <Badge>
              {recognition()?.installed ? formatBytes(recognition()?.sizeBytes ?? 0) : "—"}
            </Badge>
          </SettingsRow>
          <Show when={recognition()?.error}>
            {(error) => <StatusBanner tone="danger">{error()}</StatusBanner>}
          </Show>
        </SettingsCard>
      </SettingsSection>

      <SettingsSection title={i18n.t("settings.voice.outputTitle")}>
        <SettingsCard class="settings-card voice-runtime-card">
          <SettingsRow
            label={runtime()?.voiceName || i18n.t("settings.voice.builtIn")}
            description={i18n.t("settings.voice.offlineDescription")}
          >
            <Show when={runtime() && !runtime()!.loading}>
              <Badge tone={runtime()?.available ? "success" : "danger"}>
                {runtime()?.available
                  ? i18n.t("settings.voice.ready")
                  : i18n.t("settings.voice.unavailable")}
              </Badge>
            </Show>
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.computeMode")}
            description={i18n.t("settings.voice.computeDescription")}
          >
            <SelectField
              label={i18n.t("settings.voice.computeMode")}
              value={computeMode()}
              disabled={busy()}
              options={[
                { value: "auto", label: i18n.t("settings.voice.computeAuto") },
                { value: "direct_ml", label: "DirectML" },
                { value: "cpu", label: "CPU" },
              ]}
              onChange={(value) => void updateVoiceSettings(speed(), value as VoiceComputeMode)}
            />
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.backend")}
            description={
              runtime()?.computeDevice
                ? `${runtime()!.computeDevice!.name} · Adapter ${runtime()!.computeDevice!.deviceId} · ${runtime()!.computeDevice!.dedicatedMemoryMb} MB`
                : (runtime()?.fallbackReason ?? i18n.t("settings.voice.backendDescription"))
            }
          >
            <Badge tone={runtime()?.fallbackReason ? "warning" : "info"}>
              {runtime()?.backend === "direct_ml" ? "DirectML" : "CPU"}
            </Badge>
          </SettingsRow>
          <Show when={(runtime()?.speakerCount ?? 1) > 1}>
            <SettingsRow
              label={i18n.t("settings.voice.speakerId")}
              description={i18n
                .t("settings.voice.speakerSummary")
                .replace("{id}", String(runtime()?.speakerId ?? 0))
                .replace("{count}", String(runtime()?.speakerCount ?? 1))}
            >
              <Badge tone="info">Speaker {runtime()?.speakerId ?? 0}</Badge>
            </SettingsRow>
          </Show>
          <SettingsRow
            label={i18n.t("settings.voice.speed")}
            description={i18n.t("settings.voice.speedDescription")}
          >
            <RangeField
              label={i18n.t("settings.voice.speed")}
              min={50}
              max={200}
              step={5}
              unit="%"
              value={speed()}
              disabled={busy()}
              onInput={setSpeed}
              onCommit={(value) => void updateVoiceSettings(value, computeMode())}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.voice.muted")}>
            <Toggle
              checked={runtime()?.muted ?? false}
              label={i18n.t("settings.voice.muted")}
              onChange={(value) => void setMuted(value)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.voice.preview")}>
            <div class="voice-preview-actions">
              <Button
                size="small"
                disabled={!runtime()?.available || runtime()?.muted || busy()}
                onClick={() => void preview()}
              >
                <Play size={14} /> {i18n.t("settings.voice.preview")}
              </Button>
              <Button size="small" variant="ghost" onClick={() => void commands.stopSpeech()}>
                <Square size={12} /> {i18n.t("pet.stop")}
              </Button>
            </div>
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>

      <SettingsSection title={i18n.t("settings.voice.importTitle")}>
        <StatusBanner>{i18n.t("settings.voice.importDescription")}</StatusBanner>
        <SettingsCard class="settings-card unified-settings-card resource-import-card">
          <SettingsRow label={i18n.t("settings.resourceName")}>
            <TextField
              label={i18n.t("settings.resourceName")}
              value={name()}
              placeholder={i18n.t("settings.voice.nameExample")}
              onInput={(event) => setName(event.currentTarget.value)}
            />
          </SettingsRow>
          <div class="settings-card-actions">
            <Button variant="primary" disabled={busy()} onClick={() => void inspectModel()}>
              {i18n.t("settings.voice.inspect")}
            </Button>
          </div>
        </SettingsCard>
      </SettingsSection>

      <Show when={notice()}>
        {(value) => <StatusBanner tone={value().tone}>{value().text}</StatusBanner>}
      </Show>

      <SettingsSection title={`${i18n.t("settings.catalog")} · ${catalog().entries.length}`}>
        <ResourceList label={i18n.t("settings.voice")}>
          <For each={catalog().entries}>
            {(entry) => (
              <ResourceCard
                title={entry.name}
                subtitle={`${entry.originalFileName} · ${formatBytes(entry.sizeBytes)}`}
                current={entry.id === catalog().currentId}
                meta={
                  <div class="avatar-resource-meta">
                    <div class="avatar-badge-row">
                      <Badge tone="info">{entry.modelType}</Badge>
                      <Badge>{entry.languages.join(" / ")}</Badge>
                      <Badge>{`${entry.sampleRate.toLocaleString()} Hz`}</Badge>
                      <Show when={(entry.speakerCount ?? 1) > 1}>
                        <Badge tone="info">
                          {i18n
                            .t("settings.voice.speakerSummary")
                            .replace("{id}", String(entry.speakerId ?? 0))
                            .replace("{count}", String(entry.speakerCount ?? 1))}
                        </Badge>
                      </Show>
                      <Badge tone={entry.origin === "built_in" ? "warning" : "neutral"}>
                        {entry.origin === "built_in"
                          ? i18n.t("settings.voice.builtInOrigin")
                          : i18n.t("settings.voice.importedOrigin")}
                      </Badge>
                    </div>
                    <span>{entry.licenseSummary}</span>
                  </div>
                }
                actions={
                  <>
                    <Show
                      when={entry.id === catalog().currentId}
                      fallback={
                        <Button
                          size="small"
                          disabled={busy()}
                          onClick={() => void selectModel(entry.id)}
                        >
                          {i18n.t("common.select")}
                        </Button>
                      }
                    >
                      <Badge tone="success">{i18n.t("common.current")}</Badge>
                    </Show>
                    <Show when={!entry.protected}>
                      <Button size="small" variant="danger" onClick={() => setPendingDelete(entry)}>
                        <Trash2 size={14} /> {i18n.t("common.delete")}
                      </Button>
                    </Show>
                  </>
                }
              />
            )}
          </For>
        </ResourceList>
      </SettingsSection>

      <Dialog
        open={Boolean(inspection())}
        title={i18n.t("settings.voice.inspectionTitle")}
        description={i18n.t("settings.voice.inspectionDescription")}
        onOpenChange={(open) => {
          if (!open) void closeInspection();
        }}
      >
        <Show when={inspection()}>
          {(value) => (
            <div class="avatar-inspection-dialog">
              <div class="avatar-inspection-heading">
                <div>
                  <strong>{value().originalFileName}</strong>
                  <span>{formatBytes(value().sizeBytes)}</span>
                </div>
                <div class="avatar-badge-row">
                  <Badge tone={value().compatible ? "success" : "danger"}>
                    {value().modelType}
                  </Badge>
                  <Badge>{value().languages.join(" / ") || "Unknown"}</Badge>
                  <Badge>{`${value().sampleRate.toLocaleString()} Hz`}</Badge>
                  <Show when={value().speakerCount > 1}>
                    <Badge tone="info">{value().speakerCount.toLocaleString()} Speakers</Badge>
                  </Show>
                </div>
              </div>
              <StatusBanner tone={value().licenseWarning ? "warning" : "success"}>
                {value().licenseSummary}
              </StatusBanner>
              <div class="voice-required-files">
                <strong>{i18n.t("settings.voice.requiredFiles")}</strong>
                <ul>
                  <For each={value().requiredFiles}>
                    {(path) => (
                      <li>
                        <code>{path}</code>
                      </li>
                    )}
                  </For>
                </ul>
              </div>
              <For each={value().issues}>
                {(issue) => <StatusBanner tone="danger">{issue}</StatusBanner>}
              </For>
              <Show when={value().speakerCount > 1}>
                <SettingsRow
                  label={i18n.t("settings.voice.speakerId")}
                  description={i18n
                    .t("settings.voice.speakerIdDescription")
                    .replace("{max}", String(value().speakerCount - 1))}
                >
                  <NumberField
                    label={i18n.t("settings.voice.speakerId")}
                    min={0}
                    max={value().speakerCount - 1}
                    step={1}
                    value={speakerId()}
                    disabled={busy()}
                    onInput={(event) => setSpeakerId(Number(event.currentTarget.value))}
                  />
                </SettingsRow>
              </Show>
              <SettingsRow
                label={i18n.t("settings.voice.licenseConfirm")}
                description={i18n.t("settings.voice.licenseConfirmDescription")}
              >
                <Toggle
                  checked={licenseAcknowledged()}
                  label={i18n.t("settings.voice.licenseConfirm")}
                  onChange={setLicenseAcknowledged}
                />
              </SettingsRow>
              <div class="dialog-actions">
                <Button variant="ghost" onClick={() => void closeInspection()}>
                  {i18n.t("common.cancel")}
                </Button>
                <Button
                  variant="primary"
                  disabled={
                    !value().token ||
                    !licenseAcknowledged() ||
                    !Number.isInteger(speakerId()) ||
                    speakerId() < 0 ||
                    speakerId() >= value().speakerCount ||
                    busy()
                  }
                  onClick={() => void commitImport()}
                >
                  {i18n.t("settings.voice.confirmImport")}
                </Button>
              </div>
            </div>
          )}
        </Show>
      </Dialog>

      <Dialog
        open={Boolean(pendingDelete())}
        title={i18n.t("settings.resource.deleteTitle")}
        description={i18n
          .t("settings.resource.confirmDelete")
          .replace("{name}", pendingDelete()?.name ?? "")}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(undefined);
        }}
      >
        <div class="dialog-actions">
          <Button variant="ghost" onClick={() => setPendingDelete(undefined)}>
            {i18n.t("common.cancel")}
          </Button>
          <Button variant="danger" onClick={() => void confirmDelete()}>
            {i18n.t("common.delete")}
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

function AvatarAssessmentDetails(props: { assessment: AvatarAssessment }) {
  const i18n = useI18n();
  const stats = () => props.assessment.statistics;
  return (
    <div class="avatar-assessment-report">
      <p class="avatar-level-description">
        {avatarCompatibilityDescription(props.assessment.compatibility, i18n.locale())}
      </p>
      <Show when={(props.assessment.requirements?.length ?? 0) > 0}>
        <div class="avatar-requirement-list">
          <For each={props.assessment.requirements ?? []}>
            {(requirement) => (
              <div class="avatar-requirement-row" data-passed={requirement.passed}>
                <Badge tone={requirement.passed ? "success" : "danger"}>
                  {requirement.passed ? "✓" : "!"}
                </Badge>
                <span>{avatarRequirementLabel(requirement.requirement, i18n.locale())}</span>
                <Show when={!requirement.passed && requirement.detail}>
                  <code>{requirement.detail}</code>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
      <dl class="avatar-statistics-grid">
        <div>
          <dt>{i18n.t("settings.avatar.stats.meshes")}</dt>
          <dd>{stats().meshCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.triangles")}</dt>
          <dd>{(stats().triangleCount ?? 0).toLocaleString()}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.bones")}</dt>
          <dd>{stats().boneCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.animations")}</dt>
          <dd>{stats().animationCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.materials")}</dt>
          <dd>{stats().materialCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.textures")}</dt>
          <dd>{stats().textureCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.morphs")}</dt>
          <dd>{stats().morphTargetCount}</dd>
        </div>
      </dl>
      <Show when={props.assessment.capabilities.length > 0}>
        <div class="avatar-capability-list">
          <For each={props.assessment.capabilities}>
            {(capability) => (
              <Badge tone="info">{avatarCapabilityLabel(capability, i18n.locale())}</Badge>
            )}
          </For>
        </div>
      </Show>
      <For each={props.assessment.issues}>
        {(issue) => (
          <p class="avatar-assessment-issue" data-severity={issue.severity}>
            {avatarIssueLabel(issue.code, i18n.locale())}
          </p>
        )}
      </For>
    </div>
  );
}

function resolvedAvatarAssessment(entry: AvatarEntry): AvatarAssessment {
  return entry.assessment;
}

function avatarFormatLabel(format: AvatarFormat): string {
  if (format === "vrm0") return "VRM 0.x";
  if (format === "vrm1") return "VRM 1.0";
  return "GLB 2.0";
}

function avatarCompatibilityTone(
  compatibility: AvatarAssessment["compatibility"],
): "neutral" | "success" | "danger" {
  if (compatibility === "runtime_ready") return "success";
  return "danger";
}

function avatarCompatibilityLabel(
  compatibility: AvatarAssessment["compatibility"],
  locale: AppLocale,
): string {
  if (compatibility === "runtime_ready") return "Runtime Ready";
  return locale === "zh-CN" ? "缺少运行能力" : "Missing runtime capability";
}

function avatarCompatibilityDescription(
  compatibility: AvatarAssessment["compatibility"],
  locale: AppLocale,
): string {
  if (compatibility === "runtime_ready") {
    return locale === "zh-CN"
      ? "该 VRM 具备标准动作、神态、视线、口型和二级物理运行能力。"
      : "This VRM is ready for standard motion, expression, gaze, lip-sync, and secondary physics.";
  }
  return locale === "zh-CN"
    ? "模型缺少必要运行能力，不能导入或选择。"
    : "The model is missing required runtime capabilities and cannot be selected.";
}

function avatarRequirementLabel(requirement: string, locale: AppLocale): string {
  const zh: Record<string, string> = {
    vrm_format: "VRM 0.x / 1.0 格式",
    skinned_mesh: "完整蒙皮网格",
    complete_humanoid: "核心 Humanoid 骨骼",
    chest_bone: "胸部骨骼（可选）",
    toe_bones: "脚趾骨骼（可选）",
    finger_bones: "手指骨骼（可选）",
    standard_blinks: "Neutral 与左右眨眼",
    jaw_lip_sync: "基础嘴型（可选）",
    five_visemes: "五元音口型",
    standard_emotions: "标准情绪表情",
    look_at: "VRM LookAt",
    mtoon: "MToon 材质",
    spring_bone: "SpringBone",
    spring_collider: "SpringBone Collider",
    skin_weights: "最多四个有效蒙皮权重",
    resource_budget: "资源预算",
  };
  const en: Record<string, string> = {
    vrm_format: "VRM 0.x / 1.0 format",
    skinned_mesh: "Complete skinned mesh",
    complete_humanoid: "Core humanoid bones",
    chest_bone: "Chest bone (optional)",
    toe_bones: "Toe bones (optional)",
    finger_bones: "Finger bones (optional)",
    standard_blinks: "Neutral and left/right blink",
    jaw_lip_sync: "Basic lip sync (optional)",
    five_visemes: "Five vowel visemes",
    standard_emotions: "Standard emotion expressions",
    look_at: "VRM LookAt",
    mtoon: "MToon materials",
    spring_bone: "SpringBone",
    spring_collider: "SpringBone collider",
    skin_weights: "At most four valid skin weights",
    resource_budget: "Resource budget",
  };
  return (locale === "zh-CN" ? zh : en)[requirement] ?? requirement;
}

function avatarCapabilityLabel(capability: AvatarCapability, locale: AppLocale): string {
  const zh: Record<AvatarCapability, string> = {
    renderable_mesh: "可渲染网格",
    skinned_mesh: "蒙皮网格",
    built_in_animations: "内置动画",
    humanoid_skeleton: "人形骨骼",
    blink: "眨眼",
    viseme: "嘴型",
    look_at: "视线",
    happy_expression: "开心表情",
    sad_expression: "难过表情",
    angry_expression: "生气表情",
    spring_bone: "弹性骨骼",
    standard_motion_retarget: "标准动作",
    runtime_ready: "Runtime Ready",
    m_toon: "MToon 材质",
    spring_bone_collider: "二级物理碰撞体",
    five_finger_hands: "完整手指骨骼",
    five_visemes: "五元音口型",
    standard_expressions: "标准表情集",
    lip_sync_jaw: "基础下颌口型",
    lip_sync_five_viseme: "五元音同步口型",
  };
  const en: Record<AvatarCapability, string> = {
    renderable_mesh: "Renderable mesh",
    skinned_mesh: "Skinned mesh",
    built_in_animations: "Built-in animations",
    humanoid_skeleton: "Humanoid skeleton",
    blink: "Blink",
    viseme: "Visemes",
    look_at: "Look at",
    happy_expression: "Happy expression",
    sad_expression: "Sad expression",
    angry_expression: "Angry expression",
    spring_bone: "Spring bones",
    standard_motion_retarget: "Standard motions",
    runtime_ready: "Runtime Ready",
    m_toon: "MToon materials",
    spring_bone_collider: "Secondary-motion colliders",
    five_finger_hands: "Complete finger bones",
    five_visemes: "Five visemes",
    standard_expressions: "Standard expressions",
    lip_sync_jaw: "Jaw lip sync",
    lip_sync_five_viseme: "Five-viseme lip sync",
  };
  return (locale === "zh-CN" ? zh : en)[capability];
}

function avatarIssueLabel(code: string, locale: AppLocale): string {
  const zh: Record<string, string> = {
    invalid_glb: "文件不是有效的 GLB 2.0 模型。",
    unsupported_glb_version: "仅支持 GLB 2.0。",
    glb_length_mismatch: "模型声明长度与实际文件不一致。",
    invalid_glb_json: "模型 JSON 数据损坏。",
    valid_scene_missing: "没有找到引用可渲染网格的有效场景。",
    resource_out_of_bounds: "模型缓冲区或资源范围超出了文件边界。",
    external_resource: "模型引用了外部资源，必须将贴图和缓冲区内嵌。",
    unsupported_required_extension: "模型使用了当前运行时不支持的必需压缩扩展。",
    renderable_mesh_missing: "没有检测到可渲染三角形网格。",
    position_bounds_missing: "模型缺少有效的 POSITION 边界数据。",
    model_bounds_empty: "模型空间边界为空。",
    humanoid_mapping_incomplete: "骨骼存在，但无法可靠映射为标准人形骨骼。",
    blink_missing: "未检测到标准眨眼表情。",
    five_visemes: "模型缺少 aa/ih/ou/ee/oh 五元音口型。",
    vrm_metadata_missing: "文件扩展名为 VRM，但没有检测到 VRM 元数据。",
    legacy_asset_unreadable: "旧模型无法重新检测，已保留并隔离。",
  };
  const en: Record<string, string> = {
    invalid_glb: "The file is not a valid GLB 2.0 model.",
    unsupported_glb_version: "Only GLB 2.0 is supported.",
    glb_length_mismatch: "The declared model length does not match the file.",
    invalid_glb_json: "The model JSON is damaged.",
    valid_scene_missing: "No valid scene references a renderable mesh.",
    resource_out_of_bounds: "A model buffer or resource range exceeds the file bounds.",
    external_resource: "External resources are not allowed; embed all textures and buffers.",
    unsupported_required_extension: "The model requires an unsupported compression extension.",
    renderable_mesh_missing: "No renderable triangle mesh was detected.",
    position_bounds_missing: "Valid POSITION bounds are missing.",
    model_bounds_empty: "The model bounds are empty.",
    humanoid_mapping_incomplete: "The skeleton cannot be mapped reliably to a humanoid.",
    blink_missing: "No standard blink expression was detected.",
    five_visemes: "The model does not provide all aa/ih/ou/ee/oh vowel visemes.",
    vrm_metadata_missing: "The file uses a VRM extension but contains no VRM metadata.",
    legacy_asset_unreadable: "The legacy model could not be reassessed and was quarantined.",
  };
  return (locale === "zh-CN" ? zh : en)[code] ?? code;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatDate(epochMillis: string): string {
  const value = Number(epochMillis);
  return Number.isFinite(value) ? new Date(value).toLocaleString() : epochMillis;
}
