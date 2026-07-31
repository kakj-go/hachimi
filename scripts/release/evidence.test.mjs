import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { tmpdir } from "node:os";
import { mkdtempSync } from "node:fs";

import {
  loadStagingConfig,
  redactText,
  sha256Bytes,
  verifyArtifactManifest,
  verifyCandidateArtifacts,
  verifyEvidence,
} from "./evidence.mjs";

test("redaction removes bearer tokens, secret values, and sensitive paths", () => {
  const value = redactText("Authorization: Bearer abc.def token=plain C:\\private\\gate", [
    "C:\\private\\gate",
  ]);
  assert.equal(value.includes("abc.def"), false);
  assert.equal(value.includes("plain"), false);
  assert.equal(value.includes("C:\\private"), false);
});

test("staging config accepts references and rejects secret material", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-release-config-"));
  const valid = join(root, "valid.json");
  writeFileSync(
    valid,
    JSON.stringify({
      schemaVersion: 1,
      gateKind: "openai",
      environmentFingerprint: "openai-staging",
      secretRefs: ["credential-manager:provider:default"],
      baseUrl: "https://api.openai.com/v1",
      chatModel: "chat-staging",
      responsesModel: "responses-staging",
      embeddingModel: "embedding-staging",
      requireReasoningSummary: true,
      requireRemoteCompaction: true,
    }),
  );
  assert.equal(loadStagingConfig(valid, "openai").gateKind, "openai");
  const invalid = join(root, "invalid.json");
  writeFileSync(
    invalid,
    JSON.stringify({
      schemaVersion: 1,
      gateKind: "openai",
      environmentFingerprint: "openai-staging",
      secretRefs: ["credential-manager:release:openai"],
      apiKey: "must-not-appear",
    }),
  );
  assert.throws(() => loadStagingConfig(invalid, "openai"), /staging_config_contains_secret_field/);
});

test("Forge staging schema requires all five distinct platforms and registered credentials", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-release-forge-config-"));
  const path = join(root, "forge.json");
  const platforms = ["github", "gitlab", "gitee", "gitea", "forgejo"];
  const config = {
    schemaVersion: 1,
    gateKind: "forge",
    environmentFingerprint: "protected-forge-environments",
    secretRefs: platforms.map((platform) => `release-${platform}`),
    repositories: platforms.map((platform, index) => ({
      platformLabel: platform,
      forgeKind: platform === "gitea" || platform === "forgejo" ? "gitea_forgejo" : platform,
      apiBaseUrl: `https://${platform}.example.test/api/`,
      faultApiBaseUrl: `https://${platform}-fault.example.test/api/`,
      owner: "hachimi-gate",
      repository: `release-${platform}`,
      remoteUrlHash: String(index + 1).repeat(64),
      secretRef: `release-${platform}`,
      sourceRef: `hachimi-gate/run-${index}`,
      targetRef: "main",
      expectedCommitOid: String(index + 1).repeat(40),
      mergeSourceRef: `hachimi-gate/merge-${index}`,
      mergeCommitOid: String(index + 2).repeat(40),
      checkoutPath: `D:/staging/${platform}`,
      remoteName: "origin",
    })),
  };
  writeFileSync(path, JSON.stringify(config));
  assert.equal(loadStagingConfig(path, "forge").repositories.length, 5);
  config.repositories[4].secretRef = "unregistered";
  writeFileSync(path, JSON.stringify(config));
  assert.throws(() => loadStagingConfig(path, "forge"), /secret_ref_unregistered/);
});

test("enterprise staging schema requires each external organization and inbound evidence", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-release-enterprise-config-"));
  const path = join(root, "enterprise.json");
  const platforms = ["wecom", "ding_talk", "feishu"];
  const config = {
    schemaVersion: 1,
    gateKind: "enterprise",
    environmentFingerprint: "protected-enterprise-organizations",
    secretRefs: platforms.map((platform) => `keyring:connector:release-${platform}`),
    connections: platforms.map((platform) => ({
      platform,
      accountId: `release-${platform}`,
      credentialRef: `keyring:connector:release-${platform}`,
      departmentId: "gate-department",
      peerId: "gate-peer",
      groupId: "gate-group",
      expectInboundEvent: true,
      ...(platform === "wecom"
        ? {
            callbackPublicUrl:
              "https://wecom-gate.example.test/v1/channels/wecom/callback?account_id=release-wecom",
          }
        : {}),
    })),
  };
  writeFileSync(path, JSON.stringify(config));
  assert.equal(loadStagingConfig(path, "enterprise").connections.length, 3);
  config.connections[0].expectInboundEvent = false;
  writeFileSync(path, JSON.stringify(config));
  assert.throws(() => loadStagingConfig(path, "enterprise"), /inbound_event_required/);
});

function summary(gateKind, overrides = {}) {
  return {
    schemaVersion: 1,
    gateKind,
    status: "passed",
    version: "0.3.0-alpha.8",
    commitSha: "a".repeat(40),
    artifactSha256: [
      { name: "Hachimi.exe", sha256: "b".repeat(64) },
      { name: "Hachimi.msi", sha256: "c".repeat(64) },
      { name: "Hachimi.zip", sha256: "d".repeat(64) },
    ],
    sourceRegistrySha256: {
      openai: "c".repeat(64),
      forge: "d".repeat(64),
      enterprise: "e".repeat(64),
    },
    environmentFingerprint: "f".repeat(64),
    checks: [{ id: "conformance", status: "passed", detailsHash: "a".repeat(64) }],
    startedAtUtc: new Date().toISOString(),
    completedAtUtc: new Date().toISOString(),
    failure: null,
    ...overrides,
  };
}

test("evidence verification rejects missing, skipped, and mismatched gates", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-release-evidence-"));
  mkdirSync(join(root, "one"));
  writeFileSync(join(root, "one", "openai.summary.json"), JSON.stringify(summary("openai")));
  assert.throws(
    () => verifyEvidence(root, { required: ["openai", "forge"] }),
    /release_evidence_missing:forge/,
  );
  mkdirSync(join(root, "two"));
  writeFileSync(
    join(root, "two", "forge.summary.json"),
    JSON.stringify(summary("forge", { commitSha: "f".repeat(40) })),
  );
  assert.throws(
    () => verifyEvidence(root, { required: ["openai", "forge"] }),
    /release_evidence_commit_mismatch/,
  );
});

test("evidence verification rejects failed checks, stale evidence, and source drift", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-release-evidence-hardening-"));
  mkdirSync(join(root, "one"));
  mkdirSync(join(root, "two"));
  writeFileSync(join(root, "one", "openai.summary.json"), JSON.stringify(summary("openai")));
  writeFileSync(
    join(root, "two", "forge.summary.json"),
    JSON.stringify(
      summary("forge", {
        sourceRegistrySha256: {
          openai: "f".repeat(64),
          forge: "d".repeat(64),
          enterprise: "e".repeat(64),
        },
      }),
    ),
  );
  assert.throws(
    () => verifyEvidence(root, { required: ["openai", "forge"] }),
    /release_evidence_source_registry_mismatch/,
  );

  writeFileSync(
    join(root, "two", "forge.summary.json"),
    JSON.stringify(
      summary("forge", {
        checks: [{ id: "conformance", status: "skipped", detailsHash: "e".repeat(64) }],
      }),
    ),
  );
  assert.throws(
    () => verifyEvidence(root, { required: ["openai", "forge"] }),
    /release_evidence_check_not_passed/,
  );

  writeFileSync(
    join(root, "two", "forge.summary.json"),
    JSON.stringify(
      summary("forge", {
        startedAtUtc: "2019-12-31T23:59:00.000Z",
        completedAtUtc: "2020-01-01T00:00:00.000Z",
      }),
    ),
  );
  assert.throws(
    () => verifyEvidence(root, { required: ["openai", "forge"], now: Date.now() }),
    /release_evidence_stale:forge/,
  );

  writeFileSync(
    join(root, "two", "forge.summary.json"),
    JSON.stringify(
      summary("forge", { completedAtUtc: new Date(Date.now() + 10 * 60 * 1000).toISOString() }),
    ),
  );
  assert.throws(
    () => verifyEvidence(root, { required: ["openai", "forge"], now: Date.now() }),
    /release_evidence_stale:forge/,
  );
});

test("evidence verification rejects duplicate artifacts and success records with failure details", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-release-evidence-shape-"));
  mkdirSync(join(root, "one"));
  const duplicate = summary("openai");
  duplicate.artifactSha256.push({ name: "HACHIMI.EXE", sha256: "f".repeat(64) });
  writeFileSync(join(root, "one", "openai.summary.json"), JSON.stringify(duplicate));
  assert.throws(
    () => verifyEvidence(root, { required: ["openai"] }),
    /release_evidence_artifact_duplicate/,
  );
  writeFileSync(
    join(root, "one", "openai.summary.json"),
    JSON.stringify(summary("openai", { failure: { code: "must-not-exist" } })),
  );
  assert.throws(
    () => verifyEvidence(root, { required: ["openai"] }),
    /release_evidence_failure_present/,
  );
});

test("artifact manifest is bound to evidence, source state, and expected commit", () => {
  const evidence = summary("openai");
  const manifest = {
    schemaVersion: 1,
    version: evidence.version,
    commitSha: evidence.commitSha,
    artifactSha256: evidence.artifactSha256,
    sourceRegistrySha256: evidence.sourceRegistrySha256,
    licenseSha256: [
      { name: "LICENSE", sha256: "a".repeat(64) },
      { name: "NOTICE.md", sha256: "b".repeat(64) },
    ],
    unsigned: true,
  };
  assert.equal(
    verifyArtifactManifest(manifest, evidence, {
      expectedVersion: evidence.version,
      expectedCommit: evidence.commitSha,
    }).artifactManifest,
    manifest,
  );
  assert.throws(
    () => verifyArtifactManifest({ ...manifest, commitSha: "f".repeat(40) }, evidence),
    /release_artifact_commit_mismatch/,
  );
});

test("candidate verifier hashes downloaded files instead of trusting the manifest", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-release-candidate-"));
  mkdirSync(join(root, "docs", "references", "openai"), { recursive: true });
  mkdirSync(join(root, "docs", "references", "forge"), { recursive: true });
  mkdirSync(join(root, "docs", "references", "enterprise"), { recursive: true });
  for (const area of ["openai", "forge", "enterprise"]) {
    writeFileSync(join(root, "docs", "references", area, "registry.json"), `{"area":"${area}"}`);
  }
  writeFileSync(join(root, "LICENSE"), "license");
  writeFileSync(join(root, "NOTICE.md"), "notice");
  const artifacts = ["Hachimi.exe", "Hachimi.msi", "Hachimi.zip"].map((name) => {
    const path = join(root, name);
    writeFileSync(path, name);
    return path;
  });
  const manifest = {
    schemaVersion: 1,
    version: "0.3.0-alpha.8",
    commitSha: "a".repeat(40),
    artifactSha256: artifacts.map((path) => ({
      name: path.split(/[\\/]/).at(-1),
      sha256: sha256Bytes(readFileSync(path)),
    })),
    sourceRegistrySha256: Object.fromEntries(
      ["openai", "forge", "enterprise"].map((area) => [
        area,
        sha256Bytes(readFileSync(join(root, "docs", "references", area, "registry.json"))),
      ]),
    ),
    licenseSha256: ["LICENSE", "NOTICE.md"].map((name) => ({
      name,
      sha256: sha256Bytes(readFileSync(join(root, name))),
    })),
    unsigned: true,
  };
  assert.equal(
    verifyCandidateArtifacts(manifest, root, artifacts, {
      expectedVersion: manifest.version,
      expectedCommit: manifest.commitSha,
    }),
    manifest,
  );
  writeFileSync(artifacts[0], "tampered");
  assert.throws(
    () => verifyCandidateArtifacts(manifest, root, artifacts),
    /release_candidate_file_digest_mismatch/,
  );
  assert.throws(
    () => verifyCandidateArtifacts(manifest, root, [...artifacts, artifacts[1]]),
    /release_candidate_artifact_count_invalid/,
  );
});
