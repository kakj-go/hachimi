import { createHash } from "node:crypto";
import { readFileSync, readdirSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

export const RELEASE_EVIDENCE_SCHEMA_VERSION = 1;

const sensitiveKey = /(^|_)(api[_-]?key|token|secret|password|credential)(_|$)/i;
const allowedReferenceKey = /^(secretRef|secretRefs|credentialRef|credentialRefs)$/;

export function sha256Bytes(value) {
  return createHash("sha256").update(value).digest("hex");
}

export function sha256File(path) {
  return sha256Bytes(readFileSync(path));
}

export function redactText(value, sensitivePaths = []) {
  let output = String(value ?? "");
  for (const path of [...sensitivePaths].filter(Boolean).sort((a, b) => b.length - a.length)) {
    output = output.replaceAll(path, "<redacted-path>");
  }
  output = output
    .replace(/(authorization\s*:\s*bearer\s+)[^\s,;]+/gi, "$1<redacted>")
    .replace(/\bbearer\s+[A-Za-z0-9._~+/=-]+/gi, "Bearer <redacted>")
    .replace(
      /\b(api[_-]?key|token|secret|credential|password)\b(\s*[:=]\s*)["']?[^\s,;"']+/gi,
      "$1$2<redacted>",
    );
  return output.length > 2048 ? `${output.slice(0, 2048)}...<truncated>` : output;
}

function rejectSecretMaterial(value, path = "config") {
  if (Array.isArray(value)) {
    value.forEach((entry, index) => rejectSecretMaterial(entry, `${path}[${index}]`));
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, child] of Object.entries(value)) {
    if (sensitiveKey.test(key) && !allowedReferenceKey.test(key)) {
      throw new Error(`staging_config_contains_secret_field:${path}.${key}`);
    }
    rejectSecretMaterial(child, `${path}.${key}`);
  }
}

function rejectCredentialUrls(value, path = "config") {
  if (Array.isArray(value)) {
    value.forEach((entry, index) => rejectCredentialUrls(entry, `${path}[${index}]`));
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, child] of Object.entries(value)) {
    if (typeof child === "string" && /(url|endpoint|base)$/i.test(key)) {
      try {
        const parsed = new URL(child);
        if (parsed.username || parsed.password) {
          throw new Error(`staging_config_url_contains_credentials:${path}.${key}`);
        }
      } catch (error) {
        if (String(error.message).startsWith("staging_config_")) throw error;
      }
    }
    rejectCredentialUrls(child, `${path}.${key}`);
  }
}

function requiredString(value, code) {
  if (typeof value !== "string" || !value.trim()) throw new Error(code);
}

function boundedString(value, maximum, code) {
  requiredString(value, code);
  if (value.length > maximum) throw new Error(code);
}

function validateHttpUrl(value, code) {
  requiredString(value, code);
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error(code);
  }
  if (!["http:", "https:"].includes(parsed.protocol) || parsed.hash) throw new Error(code);
}

function validateStagingShape(config, gateKind) {
  if (gateKind === "openai") {
    for (const key of ["baseUrl", "chatModel", "responsesModel", "embeddingModel"]) {
      requiredString(config[key], `staging_config_openai_${key}_missing`);
    }
    validateHttpUrl(config.baseUrl, "staging_config_openai_base_url_invalid");
    if (config.requireReasoningSummary !== true || config.requireRemoteCompaction !== true) {
      throw new Error("staging_config_openai_required_capability_missing");
    }
    if (!config.secretRefs.includes("credential-manager:provider:default")) {
      throw new Error("staging_config_openai_secret_ref_invalid");
    }
    if (
      config.overflowProbeChars !== undefined &&
      (!Number.isSafeInteger(config.overflowProbeChars) ||
        config.overflowProbeChars < 600_000 ||
        config.overflowProbeChars > 4_000_000)
    ) {
      throw new Error("staging_config_openai_overflow_probe_invalid");
    }
    return;
  }
  if (gateKind === "forge") {
    if (!Array.isArray(config.repositories) || config.repositories.length !== 5) {
      throw new Error("staging_config_forge_repository_set_invalid");
    }
    const labels = new Set(config.repositories.map((entry) => entry?.platformLabel));
    for (const label of ["github", "gitlab", "gitee", "gitea", "forgejo"]) {
      if (!labels.has(label)) throw new Error(`staging_config_forge_platform_missing:${label}`);
    }
    for (const entry of config.repositories) {
      for (const key of [
        "apiBaseUrl",
        "faultApiBaseUrl",
        "owner",
        "repository",
        "remoteUrlHash",
        "secretRef",
        "sourceRef",
        "targetRef",
        "expectedCommitOid",
        "mergeSourceRef",
        "mergeCommitOid",
        "checkoutPath",
        "remoteName",
      ]) {
        requiredString(entry?.[key], `staging_config_forge_${key}_missing`);
      }
      validateHttpUrl(entry.apiBaseUrl, "staging_config_forge_api_base_url_invalid");
      validateHttpUrl(entry.faultApiBaseUrl, "staging_config_forge_fault_api_base_url_invalid");
      const expectedKind =
        entry.platformLabel === "gitea" || entry.platformLabel === "forgejo"
          ? "gitea_forgejo"
          : entry.platformLabel;
      if (entry.forgeKind !== expectedKind) {
        throw new Error("staging_config_forge_kind_mismatch");
      }
      if (!/^[a-z0-9._-]{1,128}$/i.test(entry.remoteName)) {
        throw new Error("staging_config_forge_remote_name_invalid");
      }
      if (
        !entry.sourceRef.startsWith("hachimi-gate/") ||
        !entry.mergeSourceRef.startsWith("hachimi-gate/")
      ) {
        throw new Error("staging_config_forge_branch_prefix_invalid");
      }
      if (!config.secretRefs.includes(entry.secretRef)) {
        throw new Error("staging_config_forge_secret_ref_unregistered");
      }
      if (!/^[a-z0-9._:-]{1,512}$/i.test(entry.secretRef)) {
        throw new Error("staging_config_forge_secret_ref_invalid");
      }
      validateDigest(entry.remoteUrlHash, "staging_config_forge_remote_hash_invalid");
      for (const oid of [entry.expectedCommitOid, entry.mergeCommitOid]) {
        if (!/^[a-f0-9]{40,64}$/i.test(oid)) throw new Error("staging_config_forge_oid_invalid");
      }
    }
    return;
  }
  if (gateKind === "enterprise") {
    if (!Array.isArray(config.connections) || config.connections.length !== 3) {
      throw new Error("staging_config_enterprise_connection_set_invalid");
    }
    const platforms = new Set(config.connections.map((entry) => entry?.platform));
    for (const platform of ["wecom", "ding_talk", "feishu"]) {
      if (!platforms.has(platform)) {
        throw new Error(`staging_config_enterprise_platform_missing:${platform}`);
      }
    }
    for (const entry of config.connections) {
      for (const key of ["accountId", "credentialRef", "departmentId", "peerId", "groupId"]) {
        requiredString(entry?.[key], `staging_config_enterprise_${key}_missing`);
      }
      if (!config.secretRefs.includes(entry.credentialRef)) {
        throw new Error("staging_config_enterprise_secret_ref_unregistered");
      }
      if (
        !/^[a-z0-9._:-]{1,512}$/i.test(entry.accountId) ||
        entry.credentialRef !== `keyring:connector:${entry.accountId}`
      ) {
        throw new Error("staging_config_enterprise_credential_ref_invalid");
      }
      if (entry.expectInboundEvent !== true) {
        throw new Error("staging_config_enterprise_inbound_event_required");
      }
      if (entry.platform === "wecom") {
        validateHttpUrl(
          entry.callbackPublicUrl,
          "staging_config_enterprise_wecom_callback_url_invalid",
        );
        const callback = new URL(entry.callbackPublicUrl);
        if (
          callback.protocol !== "https:" ||
          callback.pathname !== "/v1/channels/wecom/callback" ||
          callback.searchParams.get("account_id") !== entry.accountId
        ) {
          throw new Error("staging_config_enterprise_wecom_callback_url_invalid");
        }
      }
    }
  }
}

export function loadStagingConfig(path, expectedGateKind) {
  const config = JSON.parse(readFileSync(path, "utf8"));
  if (config.schemaVersion !== 1) throw new Error("staging_config_schema_unsupported");
  if (config.gateKind !== expectedGateKind) throw new Error("staging_config_gate_mismatch");
  if (!config.environmentFingerprint || typeof config.environmentFingerprint !== "string") {
    throw new Error("staging_config_environment_fingerprint_missing");
  }
  boundedString(
    config.environmentFingerprint,
    512,
    "staging_config_environment_fingerprint_invalid",
  );
  if (!Array.isArray(config.secretRefs) || config.secretRefs.length === 0) {
    throw new Error("staging_config_secret_refs_missing");
  }
  if (config.secretRefs.some((value) => typeof value !== "string" || !value.trim())) {
    throw new Error("staging_config_secret_ref_invalid");
  }
  if (
    new Set(config.secretRefs).size !== config.secretRefs.length ||
    config.secretRefs.some((value) => value.length > 512)
  ) {
    throw new Error("staging_config_secret_ref_invalid");
  }
  rejectSecretMaterial(config);
  rejectCredentialUrls(config);
  validateStagingShape(config, expectedGateKind);
  return config;
}

export function collectSourceRegistryDigests(workspaceRoot) {
  const entries = {
    openai: "docs/references/openai/registry.json",
    forge: "docs/references/forge/registry.json",
    enterprise: "docs/references/enterprise/registry.json",
  };
  return Object.fromEntries(
    Object.entries(entries).map(([key, relative]) => [
      key,
      sha256File(resolve(workspaceRoot, relative)),
    ]),
  );
}

export function collectArtifactDigests(paths) {
  return [...paths]
    .map((path) => resolve(path))
    .sort()
    .map((path) => ({ name: basename(path), sha256: sha256File(path) }));
}

export function writeEvidence(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function walkJson(root) {
  if (!statSync(root).isDirectory()) return root.endsWith(".json") ? [root] : [];
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    return entry.isDirectory() ? walkJson(path) : entry.name.endsWith("summary.json") ? [path] : [];
  });
}

function stableArtifacts(value) {
  return JSON.stringify(
    [...value].sort((left, right) =>
      `${left.name}:${left.sha256}`.localeCompare(`${right.name}:${right.sha256}`),
    ),
  );
}

function stableSourceRegistries(value) {
  return JSON.stringify(
    Object.entries(value ?? {}).sort(([left], [right]) => left.localeCompare(right)),
  );
}

function validateDigest(value, code) {
  if (typeof value !== "string" || !/^[a-f0-9]{64}$/i.test(value)) throw new Error(code);
}

function validateSummary(value) {
  if (typeof value.version !== "string" || !value.version) {
    throw new Error(`release_evidence_version_missing:${value.gateKind}`);
  }
  if (typeof value.commitSha !== "string" || !/^[a-f0-9]{40}$/i.test(value.commitSha)) {
    throw new Error(`release_evidence_commit_invalid:${value.gateKind}`);
  }
  const artifacts = value.artifactSha256;
  if (!Array.isArray(artifacts) || artifacts.length < 3) {
    throw new Error(`release_evidence_artifacts_incomplete:${value.gateKind}`);
  }
  const artifactNames = new Set();
  for (const artifact of artifacts) {
    if (
      !artifact ||
      typeof artifact.name !== "string" ||
      !/\.(exe|msi|zip)$/i.test(artifact.name)
    ) {
      throw new Error(`release_evidence_artifact_invalid:${value.gateKind}`);
    }
    const normalizedName = artifact.name.toLowerCase();
    if (artifactNames.has(normalizedName)) {
      throw new Error(`release_evidence_artifact_duplicate:${value.gateKind}`);
    }
    artifactNames.add(normalizedName);
    validateDigest(artifact.sha256, `release_evidence_artifact_digest_invalid:${value.gateKind}`);
  }
  for (const extension of [".exe", ".msi", ".zip"]) {
    if (!artifacts.some((artifact) => artifact.name.toLowerCase().endsWith(extension))) {
      throw new Error(`release_evidence_artifact_type_missing:${value.gateKind}:${extension}`);
    }
  }
  const registries = value.sourceRegistrySha256;
  for (const registry of ["openai", "forge", "enterprise"]) {
    validateDigest(
      registries?.[registry],
      `release_evidence_source_registry_invalid:${value.gateKind}:${registry}`,
    );
  }
  validateDigest(
    value.environmentFingerprint,
    `release_evidence_environment_fingerprint_invalid:${value.gateKind}`,
  );
  const ids = new Set();
  for (const check of value.checks) {
    if (!check || typeof check.id !== "string" || !check.id || ids.has(check.id)) {
      throw new Error(`release_evidence_check_id_invalid:${value.gateKind}`);
    }
    ids.add(check.id);
    validateDigest(check.detailsHash, `release_evidence_check_digest_invalid:${value.gateKind}`);
  }
  const started = Date.parse(value.startedAtUtc);
  const completed = Date.parse(value.completedAtUtc);
  if (!Number.isFinite(started) || !Number.isFinite(completed) || completed < started) {
    throw new Error(`release_evidence_time_invalid:${value.gateKind}`);
  }
  if (value.status === "passed" && value.failure !== null) {
    throw new Error(`release_evidence_failure_present:${value.gateKind}`);
  }
}

export function verifyArtifactManifest(manifest, evidence, options = {}) {
  if (manifest.schemaVersion !== RELEASE_EVIDENCE_SCHEMA_VERSION) {
    throw new Error("release_artifact_manifest_schema_unsupported");
  }
  if (manifest.version !== evidence.version) throw new Error("release_artifact_version_mismatch");
  if (manifest.commitSha !== evidence.commitSha)
    throw new Error("release_artifact_commit_mismatch");
  if (
    stableArtifacts(manifest.artifactSha256 ?? []) !==
    stableArtifacts(evidence.artifactSha256 ?? [])
  ) {
    throw new Error("release_artifact_digest_mismatch");
  }
  if (
    stableSourceRegistries(manifest.sourceRegistrySha256) !==
    stableSourceRegistries(evidence.sourceRegistrySha256)
  ) {
    throw new Error("release_artifact_source_registry_mismatch");
  }
  if (options.expectedVersion && evidence.version !== options.expectedVersion) {
    throw new Error("release_expected_version_mismatch");
  }
  if (options.expectedCommit && evidence.commitSha !== options.expectedCommit) {
    throw new Error("release_expected_commit_mismatch");
  }
  if (manifest.unsigned !== true) throw new Error("release_artifact_signature_status_missing");
  if (!Array.isArray(manifest.licenseSha256) || manifest.licenseSha256.length < 2) {
    throw new Error("release_artifact_license_digest_missing");
  }
  return { ...evidence, artifactManifest: manifest };
}

export function verifyCandidateArtifacts(manifest, workspaceRoot, artifactPaths, options = {}) {
  validateCandidateArtifactSet(artifactPaths);
  if (manifest.schemaVersion !== RELEASE_EVIDENCE_SCHEMA_VERSION) {
    throw new Error("release_artifact_manifest_schema_unsupported");
  }
  if (options.expectedVersion && manifest.version !== options.expectedVersion) {
    throw new Error("release_expected_version_mismatch");
  }
  if (options.expectedCommit && manifest.commitSha !== options.expectedCommit) {
    throw new Error("release_expected_commit_mismatch");
  }
  if (
    stableArtifacts(manifest.artifactSha256 ?? []) !==
    stableArtifacts(collectArtifactDigests(artifactPaths))
  ) {
    throw new Error("release_candidate_file_digest_mismatch");
  }
  if (
    stableSourceRegistries(manifest.sourceRegistrySha256) !==
    stableSourceRegistries(collectSourceRegistryDigests(workspaceRoot))
  ) {
    throw new Error("release_candidate_source_registry_mismatch");
  }
  const licenses = [resolve(workspaceRoot, "LICENSE"), resolve(workspaceRoot, "NOTICE.md")];
  if (
    stableArtifacts(manifest.licenseSha256 ?? []) !==
    stableArtifacts(collectArtifactDigests(licenses))
  ) {
    throw new Error("release_candidate_license_digest_mismatch");
  }
  if (manifest.unsigned !== true) throw new Error("release_artifact_signature_status_missing");
  return manifest;
}

export function validateCandidateArtifactSet(artifactPaths) {
  if (!Array.isArray(artifactPaths) || artifactPaths.length !== 3) {
    throw new Error("release_candidate_artifact_count_invalid");
  }
  for (const extension of [".exe", ".msi", ".zip"]) {
    const count = artifactPaths.filter((path) => path.toLowerCase().endsWith(extension)).length;
    if (count !== 1) {
      throw new Error(`release_candidate_artifact_type_count_invalid:${extension}:${count}`);
    }
  }
}

export function verifyEvidence(root, options = {}) {
  const required = options.required ?? [
    "openai",
    "forge",
    "enterprise",
    "windows_standard_user",
    "windows_elevated",
  ];
  const maxAgeHours = options.maxAgeHours ?? 168;
  const now = options.now ?? Date.now();
  if (!Number.isFinite(maxAgeHours) || maxAgeHours <= 0 || !Number.isFinite(now)) {
    throw new Error("release_evidence_time_window_invalid");
  }
  const summaries = walkJson(resolve(root)).map((path) => ({
    path,
    value: JSON.parse(readFileSync(path, "utf8")),
  }));
  const selected = new Map();
  for (const item of summaries) {
    const value = item.value;
    if (value.schemaVersion !== RELEASE_EVIDENCE_SCHEMA_VERSION || !value.gateKind) continue;
    const previous = selected.get(value.gateKind);
    if (!previous || Date.parse(value.completedAtUtc) > Date.parse(previous.value.completedAtUtc)) {
      selected.set(value.gateKind, item);
    }
  }
  for (const gate of required) {
    if (!selected.has(gate)) throw new Error(`release_evidence_missing:${gate}`);
  }
  const values = required.map((gate) => selected.get(gate).value);
  for (const value of values) {
    if (value.status !== "passed") throw new Error(`release_evidence_not_passed:${value.gateKind}`);
    if (!Array.isArray(value.checks) || value.checks.length === 0) {
      throw new Error(`release_evidence_checks_missing:${value.gateKind}`);
    }
    if (value.checks.some((check) => check.status !== "passed")) {
      throw new Error(`release_evidence_check_not_passed:${value.gateKind}`);
    }
    validateSummary(value);
    const completed = Date.parse(value.completedAtUtc);
    if (
      !Number.isFinite(completed) ||
      completed > now + 5 * 60 * 1000 ||
      now - completed > maxAgeHours * 60 * 60 * 1000
    ) {
      throw new Error(`release_evidence_stale:${value.gateKind}`);
    }
  }
  const baseline = values[0];
  const artifacts = stableArtifacts(baseline.artifactSha256 ?? []);
  const sourceRegistries = stableSourceRegistries(baseline.sourceRegistrySha256);
  if (sourceRegistries === "[]") throw new Error("release_evidence_source_registry_missing");
  for (const value of values.slice(1)) {
    if (value.version !== baseline.version) throw new Error("release_evidence_version_mismatch");
    if (value.commitSha !== baseline.commitSha) throw new Error("release_evidence_commit_mismatch");
    if (stableArtifacts(value.artifactSha256 ?? []) !== artifacts) {
      throw new Error("release_evidence_artifact_mismatch");
    }
    if (stableSourceRegistries(value.sourceRegistrySha256) !== sourceRegistries) {
      throw new Error("release_evidence_source_registry_mismatch");
    }
  }
  return {
    schemaVersion: RELEASE_EVIDENCE_SCHEMA_VERSION,
    status: "passed",
    version: baseline.version,
    commitSha: baseline.commitSha,
    artifactSha256: baseline.artifactSha256,
    sourceRegistrySha256: baseline.sourceRegistrySha256,
    generatedAtUtc: new Date(now).toISOString(),
    gates: values.map((value) => ({
      gateKind: value.gateKind,
      completedAtUtc: value.completedAtUtc,
      environmentFingerprint: value.environmentFingerprint,
      checks: value.checks,
    })),
  };
}
