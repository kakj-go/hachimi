import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  AvatarCatalogSnapshot,
  AvatarImportCommitRequest,
  AvatarImportInspection,
  AvatarRuntimeAsset,
  BootstrapState,
  FrontendLogEntry,
  InteractiveRegionsUpdate,
  LlmSettingsInput,
  LlmSettingsView,
  LlmTestResult,
  InteractionMotionBindingUpdateRequest,
  MotionAssetBindingsClearRequest,
  MotionBindingResetRequest,
  MotionCatalogSnapshot,
  MotionEnabledUpdateRequest,
  MotionImportCommitRequest,
  MotionImportInspection,
  MotionMetadataUpdateRequest,
  MotionRuntimeAsset,
  PetContextMenuRequest,
  PetTurnRequest,
  SpeechRecognitionRuntimeState,
  SpeechRecognitionSettingsInput,
  ThemeScheme,
  VoiceCatalogSnapshot,
  VoiceImportCommitRequest,
  VoiceModelInspection,
  VoiceRuntimeState,
  VoiceSettingsInput,
  WorkbenchRoute,
} from "./generated";

export interface CommandFailure {
  code: string;
  message: string;
}

export const commands = {
  getBootstrapState: () => invoke<BootstrapState>("get_bootstrap_state"),
  frontendReady: () => invoke<void>("frontend_ready"),
  writeFrontendLog: (entry: FrontendLogEntry) => invoke<void>("write_frontend_log", { entry }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  updateSettings: (settings: AppSettings) => invoke<AppSettings>("update_settings", { settings }),
  resetLocalData: () => invoke<void>("reset_local_data"),
  importThemeProfile: (scheme: ThemeScheme) =>
    invoke<AppSettings | null>("import_theme_profile", { scheme }),
  copyThemeProfile: (profileId: string) => invoke<void>("copy_theme_profile", { profileId }),
  resetThemeProfile: (profileId: string) =>
    invoke<AppSettings>("reset_theme_profile", { profileId }),
  deleteThemeProfile: (profileId: string) =>
    invoke<AppSettings>("delete_theme_profile", { profileId }),
  setInteractiveRegions: (update: InteractiveRegionsUpdate) =>
    invoke<void>("set_interactive_regions", { update }),
  setAlwaysOnTop: (enabled: boolean) => invoke<AppSettings>("set_always_on_top", { enabled }),
  startPetDragging: () => invoke<void>("start_pet_dragging"),
  hidePetWindow: () => invoke<void>("hide_pet_window"),
  showPetContextMenu: (request: PetContextMenuRequest) =>
    invoke<void>("show_pet_context_menu", { request }),
  openWorkbench: (route: WorkbenchRoute) => invoke<void>("open_workbench", { route }),
  hideWorkbench: () => invoke<void>("hide_workbench"),
  minimizeWorkbench: () => invoke<void>("minimize_workbench"),
  toggleMaximizeWorkbench: () => invoke<void>("toggle_maximize_workbench"),
  startWorkbenchDragging: () => invoke<void>("start_workbench_dragging"),
  startWorkbenchResize: (direction: string) =>
    invoke<void>("start_workbench_resize", { direction }),
  getLlmSettings: () => invoke<LlmSettingsView>("get_llm_settings"),
  saveLlmSettings: (input: LlmSettingsInput) =>
    invoke<LlmSettingsView>("save_llm_settings", { input }),
  saveAndTestLlmSettings: (input: LlmSettingsInput) =>
    invoke<LlmTestResult>("save_and_test_llm_settings", { input }),
  listAvatarModels: () => invoke<AvatarCatalogSnapshot>("list_avatar_models"),
  inspectAvatarModel: () => invoke<AvatarImportInspection | null>("inspect_avatar_model"),
  commitAvatarModelImport: (request: AvatarImportCommitRequest) =>
    invoke<AvatarCatalogSnapshot>("commit_avatar_model_import", { request }),
  cancelAvatarModelImport: (token: string) => invoke<void>("cancel_avatar_model_import", { token }),
  selectAvatarModel: (id: string) =>
    invoke<AvatarCatalogSnapshot>("select_avatar_model", { request: { id } }),
  deleteAvatarModel: (id: string) =>
    invoke<AvatarCatalogSnapshot>("delete_avatar_model", { request: { id } }),
  getCurrentAvatarAsset: () => invoke<AvatarRuntimeAsset | null>("get_current_avatar_asset"),
  getAvatarRuntimeAsset: (id: string) =>
    invoke<AvatarRuntimeAsset | null>("get_avatar_runtime_asset", { request: { id } }),
  listMotionCatalog: () => invoke<MotionCatalogSnapshot>("list_motion_catalog"),
  inspectMotionFile: () => invoke<MotionImportInspection | null>("inspect_motion_file"),
  commitMotionImport: (request: MotionImportCommitRequest) =>
    invoke<MotionCatalogSnapshot>("commit_motion_import", { request }),
  cancelMotionImport: (token: string) => invoke<void>("cancel_motion_import", { token }),
  updateMotionMetadata: (request: MotionMetadataUpdateRequest) =>
    invoke<MotionCatalogSnapshot>("update_motion_metadata", { request }),
  deleteUserMotion: (id: string) =>
    invoke<MotionCatalogSnapshot>("delete_user_motion", { request: { id } }),
  setInteractionMotionBinding: (request: InteractionMotionBindingUpdateRequest) =>
    invoke<MotionCatalogSnapshot>("set_interaction_motion_binding", { request }),
  clearMotionInteractionBindings: (request: MotionAssetBindingsClearRequest) =>
    invoke<MotionCatalogSnapshot>("clear_motion_interaction_bindings", { request }),
  setMotionEnabled: (request: MotionEnabledUpdateRequest) =>
    invoke<MotionCatalogSnapshot>("set_motion_enabled", { request }),
  resetMotionBindings: () => invoke<MotionCatalogSnapshot>("reset_motion_bindings"),
  resetMotionBinding: (request: MotionBindingResetRequest) =>
    invoke<MotionCatalogSnapshot>("reset_motion_binding", { request }),
  getMotionRuntimeAsset: (id: string) =>
    invoke<MotionRuntimeAsset | null>("get_motion_runtime_asset", { request: { id } }),
  startPetTurn: (request: PetTurnRequest) => invoke<void>("start_pet_turn", { request }),
  cancelPetTurn: () => invoke<void>("cancel_pet_turn"),
  getVoiceRuntimeState: () => invoke<VoiceRuntimeState>("get_voice_runtime_state"),
  getSpeechRecognitionState: () =>
    invoke<SpeechRecognitionRuntimeState>("get_speech_recognition_state"),
  updateSpeechRecognitionSettings: (input: SpeechRecognitionSettingsInput) =>
    invoke<SpeechRecognitionRuntimeState>("update_speech_recognition_settings", { input }),
  listVoiceModels: () => invoke<VoiceCatalogSnapshot>("list_voice_models"),
  inspectVoiceModel: () => invoke<VoiceModelInspection | null>("inspect_voice_model"),
  commitVoiceModelImport: (request: VoiceImportCommitRequest) =>
    invoke<VoiceCatalogSnapshot>("commit_voice_model_import", { request }),
  cancelVoiceModelImport: (token: string) => invoke<void>("cancel_voice_model_import", { token }),
  selectVoiceModel: (id: string) =>
    invoke<VoiceCatalogSnapshot>("select_voice_model", { request: { id } }),
  deleteVoiceModel: (id: string) =>
    invoke<VoiceCatalogSnapshot>("delete_voice_model", { request: { id } }),
  updateVoiceSettings: (input: VoiceSettingsInput) =>
    invoke<VoiceRuntimeState>("update_voice_settings", { input }),
  setMuted: (muted: boolean) => invoke<VoiceRuntimeState>("set_muted", { muted }),
  previewDefaultVoice: () => invoke<VoiceRuntimeState>("preview_default_voice"),
  stopSpeech: () => invoke<VoiceRuntimeState>("stop_speech"),
  recognizePetSpeech: () => invoke<string>("recognize_pet_speech"),
  exitApp: () => invoke<void>("exit_app"),
};

export function commandFailure(error: unknown): CommandFailure {
  if (typeof error === "object" && error !== null && "message" in error) {
    const value = error as { code?: unknown; message: unknown };
    return {
      code: typeof value.code === "string" ? value.code : "command_failed",
      message: String(value.message),
    };
  }
  return { code: "command_failed", message: String(error) };
}

let frontendLoggingInstalled = false;

function formatLogArgument(value: unknown): string {
  if (value instanceof Error) return value.stack || `${value.name}: ${value.message}`;
  if (typeof value === "string") return value;
  if (value === null || value === undefined) return String(value);
  if (typeof value === "number" || typeof value === "boolean" || typeof value === "bigint") {
    return String(value);
  }
  return Object.prototype.toString.call(value);
}

export function installFrontendLogging(): void {
  if (frontendLoggingInstalled || typeof window === "undefined") return;
  frontendLoggingInstalled = true;
  const emit = (level: FrontendLogEntry["level"], values: unknown[]) => {
    const message = values.map(formatLogArgument).join(" ").slice(0, 4_096);
    if (message) void commands.writeFrontendLog({ level, message }).catch(() => undefined);
  };
  const originalInfo = console.info.bind(console);
  const originalWarn = console.warn.bind(console);
  const originalError = console.error.bind(console);
  console.info = (...values: unknown[]) => {
    originalInfo(...values);
    emit("info", values);
  };
  console.warn = (...values: unknown[]) => {
    originalWarn(...values);
    emit("warn", values);
  };
  console.error = (...values: unknown[]) => {
    originalError(...values);
    emit("error", values);
  };
  window.addEventListener("error", (event) => {
    emit("error", [event.error instanceof Error ? event.error : event.message]);
  });
  window.addEventListener("unhandledrejection", (event) => {
    emit("error", [event.reason]);
  });
  emit("info", ["frontend logging initialized"]);
}
