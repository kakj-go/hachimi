import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { deriveMsiVersion } from "./build-installers.mjs";

export function readReleaseVersions(root) {
  const packageJson = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
  const tauri = JSON.parse(
    readFileSync(resolve(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"),
  );
  const cargo = readFileSync(resolve(root, "Cargo.toml"), "utf8");
  const workspaceVersion = cargo.match(
    /\[workspace\.package\][\s\S]*?\bversion\s*=\s*"([^"]+)"/,
  )?.[1];
  if (!workspaceVersion) throw new Error("release_version_cargo_missing");
  const workspaceLicense = cargo.match(
    /\[workspace\.package\][\s\S]*?\blicense\s*=\s*"([^"]+)"/,
  )?.[1];
  const packageFiles = [
    "package.json",
    "apps/desktop/web/package.json",
    ...readdirSync(resolve(root, "packages"), { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => `packages/${entry.name}/package.json`)
      .filter((path) => existsSync(resolve(root, path))),
  ];
  const jsPackages = packageFiles.map((path) => {
    const value = JSON.parse(readFileSync(resolve(root, path), "utf8"));
    return { path, name: value.name, version: value.version, license: value.license };
  });
  return {
    package: packageJson.version,
    tauri: tauri.version,
    cargo: workspaceVersion,
    packageLicense: packageJson.license,
    cargoLicense: workspaceLicense,
    tauriConfig: tauri,
    jsPackages,
  };
}

export function verifyReleaseVersion(root, expectedTag = "") {
  const versions = readReleaseVersions(root);
  const unique = new Set([versions.package, versions.tauri, versions.cargo]);
  if (unique.size !== 1) throw new Error(`release_version_mismatch:${JSON.stringify(versions)}`);
  if (versions.packageLicense !== "Apache-2.0" || versions.cargoLicense !== "Apache-2.0") {
    throw new Error("release_license_metadata_mismatch");
  }
  const version = versions.package;
  const msiVersion = deriveMsiVersion(version);
  if (versions.tauriConfig.bundle?.targets?.includes("msi") !== true) {
    throw new Error("release_msi_bundle_target_missing");
  }
  const packageScripts = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8")).scripts;
  if (packageScripts?.["build:installer"] !== "node scripts/release/build-installers.mjs") {
    throw new Error("release_installer_builder_mismatch");
  }
  for (const pkg of versions.jsPackages) {
    if (pkg.version !== version) throw new Error(`release_js_package_version_mismatch:${pkg.path}`);
    if (pkg.license !== "Apache-2.0") {
      throw new Error(`release_js_package_license_mismatch:${pkg.path}`);
    }
  }
  if (expectedTag && expectedTag !== `v${version}`) {
    throw new Error(`release_tag_version_mismatch:${expectedTag}:v${version}`);
  }
  for (const required of ["LICENSE", "NOTICE.md", "Cargo.lock", "pnpm-lock.yaml"]) {
    if (!existsSync(resolve(root, required)))
      throw new Error(`release_required_file_missing:${required}`);
  }
  const attributes = readFileSync(resolve(root, ".gitattributes"), "utf8");
  if (!attributes.includes("* text=auto eol=lf")) {
    throw new Error("release_cross_platform_line_endings_missing");
  }
  const notice = readFileSync(resolve(root, "NOTICE.md"), "utf8");
  for (const marker of [
    "Apache License, Version 2.0",
    "non-commercial distributions",
    "assets/avatar-default/2639776812528692620/manifest.json",
    "THIRD-PARTY-NOTICES.md",
  ]) {
    if (!notice.includes(marker)) throw new Error(`release_notice_boundary_missing:${marker}`);
  }
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps", "--locked"], {
      cwd: root,
      encoding: "utf8",
      windowsHide: true,
    }),
  );
  for (const pkg of metadata.packages.filter((pkg) => pkg.source === null)) {
    if (pkg.version !== version) throw new Error(`release_workspace_version_mismatch:${pkg.name}`);
  }
  const tauriText = readFileSync(resolve(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8");
  if (versions.tauriConfig.bundle?.licenseFile !== "../../../LICENSE") {
    throw new Error("release_bundle_license_file_missing");
  }
  for (const resource of ['"../../../LICENSE": "LICENSE"', '"../../../NOTICE.md": "NOTICE.md"']) {
    if (!tauriText.includes(resource)) throw new Error("release_license_resource_missing");
  }
  for (const resource of [
    "assets/avatar-default/2639776812528692620/NOTICE.md",
    "resources/ai-models/THIRD-PARTY-NOTICES.md",
    "resources/ai-models/speech-to-text/",
    "resources/ai-models/text-to-speech/",
  ]) {
    if (!tauriText.includes(resource))
      throw new Error("release_third_party_notice_resource_missing");
  }
  const portable = readFileSync(resolve(root, "scripts/build-portable.ps1"), "utf8");
  for (const file of ['"LICENSE"', '"NOTICE.md"', '"resources"']) {
    if (!portable.includes(file)) throw new Error("release_portable_license_resource_missing");
  }
  const packagedLicenseGate = readFileSync(
    resolve(root, "scripts/release/test-package-licenses.ps1"),
    "utf8",
  );
  for (const marker of [
    "source_license",
    "default_avatar_notice",
    "speech_third_party_notices",
    "release_package_default_avatar_missing",
  ]) {
    if (!packagedLicenseGate.includes(marker)) {
      throw new Error("release_package_license_gate_incomplete");
    }
  }
  const workbench = readFileSync(resolve(root, "packages/workbench/src/index.tsx"), "utf8");
  if (!workbench.includes(`<Badge>v${version}</Badge>`)) {
    throw new Error("release_about_version_mismatch");
  }
  for (const path of ["packages/workbench/src/home.tsx"]) {
    if (!readFileSync(resolve(root, path), "utf8").includes(`hachimi-desktop/${version}`)) {
      throw new Error(`release_client_version_mismatch:${path}`);
    }
  }
  return {
    version,
    msiVersion,
    versions: { package: versions.package, tauri: versions.tauri, cargo: versions.cargo },
    license: "Apache-2.0",
    workspacePackages: metadata.packages.length,
    jsPackages: versions.jsPackages.length,
  };
}

const isCli = process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1]);
if (isCli) {
  const root = resolve(fileURLToPath(new URL("../..", import.meta.url)));
  const tagArgument = process.argv.findIndex((value) => value === "--tag");
  const tag =
    tagArgument >= 0
      ? (process.argv[tagArgument + 1] ?? "")
      : process.env.GITHUB_REF_TYPE === "tag"
        ? process.env.GITHUB_REF_NAME
        : "";
  const result = verifyReleaseVersion(root, tag);
  process.stdout.write(`${JSON.stringify(result)}\n`);
}
