import { copyFileSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import process from "node:process";

const workspaceRoot = resolve(import.meta.dirname, "..");
const mode = process.argv[2] ?? "dev";
const profiles = mode === "both" ? [false, true] : [mode === "release"];
const sidecars = [
  { package: "hachimi-workspace", binary: "hachimi-workspace-worker" },
  { package: "hachimi-sandbox", binary: "hachimi-sandbox-launcher" },
  { package: "hachimi-sandbox", binary: "hachimi-sandbox-canary" },
  { package: "hachimi-sandbox", binary: "hachimi-sandbox-attest" },
  { package: "hachimi-sandbox", binary: "hachimi-sandbox-setup" },
];

const rustc = spawnSync("rustc", ["-vV"], {
  cwd: workspaceRoot,
  encoding: "utf8",
  stdio: ["ignore", "pipe", "inherit"],
});
if (rustc.status !== 0) process.exit(rustc.status ?? 1);
const host = rustc.stdout
  .split(/\r?\n/u)
  .find((line) => line.startsWith("host: "))
  ?.slice("host: ".length)
  .trim();
if (!host) throw new Error("rustc did not report a host target triple");

const targetRoot = process.env.CARGO_TARGET_DIR
  ? resolve(workspaceRoot, process.env.CARGO_TARGET_DIR)
  : resolve(workspaceRoot, "target");
const binaries = resolve(workspaceRoot, "apps", "desktop", "src-tauri", "binaries");
const samplePluginBin = resolve(workspaceRoot, "assets", "plugins", "sample-crm", "bin");
const suffix = process.platform === "win32" ? ".exe" : "";
mkdirSync(binaries, { recursive: true });
mkdirSync(samplePluginBin, { recursive: true });

for (const release of profiles) {
  for (const sidecar of sidecars) {
    const cargoArgs = ["build", "-p", sidecar.package, "--bin", sidecar.binary];
    if (release) cargoArgs.push("--release");
    const cargo = spawnSync("cargo", cargoArgs, {
      cwd: workspaceRoot,
      stdio: "inherit",
    });
    if (cargo.status !== 0) process.exit(cargo.status ?? 1);
    const profileDirectory = release ? "release" : "debug";
    const executableName = `${sidecar.binary}${suffix}`;
    const source = resolve(targetRoot, profileDirectory, executableName);
    const destination = resolve(binaries, `${sidecar.binary}-${host}${suffix}`);
    copyFileSync(source, destination);
    process.stdout.write(
      `Prepared ${profileDirectory} sidecar ${sidecar.binary}: ${source}${release ? " (externalBin source)" : ""}\n`,
    );
  }
  const fixtureBinary = "hachimi-sidecar-fixture";
  const fixtureArgs = ["build", "-p", "hachimi-extensions", "--bin", fixtureBinary];
  if (release) fixtureArgs.push("--release");
  const fixtureBuild = spawnSync("cargo", fixtureArgs, {
    cwd: workspaceRoot,
    stdio: "inherit",
  });
  if (fixtureBuild.status !== 0) process.exit(fixtureBuild.status ?? 1);
  const profileDirectory = release ? "release" : "debug";
  const fixtureSource = resolve(targetRoot, profileDirectory, `${fixtureBinary}${suffix}`);
  const fixtureDestination = resolve(samplePluginBin, `${fixtureBinary}${suffix}`);
  copyFileSync(fixtureSource, fixtureDestination);
  process.stdout.write(
    `Prepared ${profileDirectory} sample Plugin sidecar ${fixtureBinary}: ${fixtureSource}\n`,
  );
}
process.stdout.write(`Prepared ${sidecars.length} external sidecars in ${binaries}\n`);
