import { spawn } from "node:child_process";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { delimiter, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  collectArtifactDigests,
  collectSourceRegistryDigests,
  loadStagingConfig,
  redactText,
  sha256Bytes,
  writeEvidence,
} from "./evidence.mjs";

const definitions = {
  openai: {
    configEnv: "HACHIMI_STAGING_OPENAI_CONFIG",
    checks: [
      {
        id: "openai_deterministic_provider_conformance",
        cargo: ["test", "-p", "hachimi-llm", "--lib"],
      },
      {
        id: "openai_deterministic_remote_context_fallback",
        cargo: ["test", "-p", "hachimi-agent", "compaction::tests::"],
      },
      {
        id: "openai_real_provider_conformance",
        cargo: [
          "test",
          "-p",
          "hachimi-llm",
          "--test",
          "staging_openai",
          "--",
          "--ignored",
          "--exact",
          "--nocapture",
          "--test-threads=1",
        ],
      },
    ],
  },
  forge: {
    configEnv: "HACHIMI_STAGING_FORGE_CONFIG",
    checks: [
      {
        id: "forge_deterministic_adapter_conformance",
        cargo: ["test", "-p", "hachimi-forge", "--lib"],
      },
      {
        id: "forge_deterministic_side_effect_reconciliation",
        cargo: [
          "test",
          "-p",
          "hachimi-storage",
          "forge_ledger_is_idempotent_and_reconciliation_safe",
        ],
      },
      {
        id: "git_standard_remote_push",
        cargo: [
          "test",
          "-p",
          "hachimi-workspace",
          "standard_remote_push_is_oid_and_url_hash_fenced",
        ],
      },
      {
        id: "git_real_remote_fetch_push",
        cargo: [
          "test",
          "-p",
          "hachimi-workspace",
          "--test",
          "staging_git",
          "--",
          "--ignored",
          "--exact",
          "--nocapture",
          "--test-threads=1",
        ],
      },
      {
        id: "forge_real_mutation_conformance",
        cargo: [
          "test",
          "-p",
          "hachimi-forge",
          "--test",
          "staging_forge",
          "--",
          "--ignored",
          "--exact",
          "--nocapture",
          "--test-threads=1",
        ],
      },
    ],
  },
  enterprise: {
    configEnv: "HACHIMI_STAGING_ENTERPRISE_CONFIG",
    checks: [
      {
        id: "enterprise_deterministic_transport_security",
        cargo: ["test", "-p", "hachimi-enterprise", "--lib"],
      },
      {
        id: "enterprise_deterministic_attachment_fencing",
        cargo: ["test", "-p", "hachimi-extensions", "enterprise_attachment::tests::"],
      },
      {
        id: "enterprise_deterministic_gateway_reconciliation",
        cargo: [
          "test",
          "-p",
          "hachimi-gateway",
          "loopback_auth_dedup_and_restart_reconciliation_are_durable",
        ],
      },
      {
        id: "enterprise_real_wecom_gateway_callback",
        cargo: [
          "test",
          "-p",
          "hachimi-gateway",
          "--test",
          "staging_enterprise_gateway",
          "--",
          "--ignored",
          "--exact",
          "--nocapture",
          "--test-threads=1",
        ],
      },
      {
        id: "enterprise_real_tenant_conformance",
        cargo: [
          "test",
          "-p",
          "hachimi-enterprise",
          "--test",
          "staging_enterprise",
          "--",
          "--ignored",
          "--exact",
          "--nocapture",
          "--test-threads=1",
        ],
      },
    ],
  },
};

const gateKind = process.argv[2];
const definition = definitions[gateKind];
if (!definition) throw new Error(`staging_gate_unknown:${gateKind ?? "missing"}`);

const workspaceRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const startedAtUtc = new Date().toISOString();
const runId = `${startedAtUtc.replaceAll(/[-:.]/g, "").replace("Z", "Z")}-${gateKind}`;
const output = resolve(
  workspaceRoot,
  "target",
  "release-evidence",
  runId,
  `${gateKind}.summary.json`,
);
const version = JSON.parse(readFileSync(resolve(workspaceRoot, "package.json"), "utf8")).version;
const commitSha = execFileSync("git", ["rev-parse", "HEAD"], {
  cwd: workspaceRoot,
  encoding: "utf8",
  windowsHide: true,
}).trim();
const configuredArtifacts = (process.env.HACHIMI_RELEASE_ARTIFACTS ?? "")
  .split(delimiter)
  .map((value) => value.trim())
  .filter(Boolean)
  .map((value) => resolve(value));

let status = "failed";
let failure = null;
let config = null;
const checks = definition.checks.map((check) => ({
  id: check.id,
  status: "failed",
  detailsHash: sha256Bytes("not-run"),
}));
try {
  const configPath = process.env[definition.configEnv];
  if (!configPath) throw new Error(`staging_config_environment_missing:${definition.configEnv}`);
  config = loadStagingConfig(resolve(configPath), gateKind);
  if (configuredArtifacts.length === 0 || configuredArtifacts.some((path) => !existsSync(path))) {
    throw new Error("release_artifact_missing");
  }
  for (let index = 0; index < definition.checks.length; index += 1) {
    const check = definition.checks[index];
    const child = spawn("cargo", check.cargo, {
      cwd: workspaceRoot,
      env: { ...process.env, HACHIMI_STAGING_ACTIVE_GATE: gateKind },
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let captured = "";
    child.stdout.on("data", (chunk) => (captured += chunk.toString()));
    child.stderr.on("data", (chunk) => (captured += chunk.toString()));
    const exitCode = await new Promise((accept, reject) => {
      child.once("error", reject);
      child.once("close", accept);
    });
    const sanitized = redactText(captured, [workspaceRoot, resolve(configPath)]);
    checks[index].detailsHash = sha256Bytes(sanitized);
    process.stdout.write(`${sanitized}\n`);
    if (exitCode !== 0) throw new Error(`staging_check_failed:${check.id}:${exitCode}`);
    checks[index].status = "passed";
  }
  status = "passed";
} catch (error) {
  failure = {
    code: String(error.message).split(":", 1)[0] || "staging_gate_failed",
    message: redactText(error.message, [workspaceRoot]),
  };
}

const summary = {
  schemaVersion: 1,
  gateKind,
  status,
  version,
  commitSha,
  artifactSha256:
    configuredArtifacts.length > 0 && configuredArtifacts.every((path) => existsSync(path))
      ? collectArtifactDigests(configuredArtifacts)
      : [],
  sourceRegistrySha256: collectSourceRegistryDigests(workspaceRoot),
  environmentFingerprint: sha256Bytes(config?.environmentFingerprint ?? "unavailable"),
  checks,
  startedAtUtc,
  completedAtUtc: new Date().toISOString(),
  failure,
};
writeEvidence(output, summary);
process.stdout.write(`${output}\n`);
if (status !== "passed") process.exitCode = 1;
