import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const cargoExecutableName = process.platform === "win32" ? "cargo.exe" : "cargo";
const rustTool = process.argv[2];
const rustToolArguments = process.argv.slice(3);

if (rustTool !== "cargo" && rustTool !== "tauri") {
  console.error("Usage: node scripts/run-with-rust.mjs <cargo|tauri> [...arguments]");
  process.exit(2);
}

function cargoBinCandidates() {
  const candidates = [];
  if (process.env.CARGO_HOME) candidates.push(join(process.env.CARGO_HOME, "bin"));
  candidates.push(join(homedir(), ".cargo", "bin"));
  candidates.push(...(process.env.PATH ?? "").split(delimiter).filter(Boolean));
  return [...new Set(candidates)];
}

const cargoBin = cargoBinCandidates().find((candidate) =>
  existsSync(join(candidate, cargoExecutableName)),
);

if (!cargoBin) {
  console.error(
    "Cargo was not found. Install Rust with rustup, or set CARGO_HOME/PATH before building Hachimi.",
  );
  process.exit(127);
}

const childEnvironment = {
  ...process.env,
  PATH: `${cargoBin}${delimiter}${process.env.PATH ?? ""}`,
};

function cargoTestProfileDirectory() {
  const profileIndex = rustToolArguments.findIndex(
    (argument) => argument === "--profile" || argument.startsWith("--profile="),
  );
  if (profileIndex >= 0) {
    const argument = rustToolArguments[profileIndex];
    const profile = argument.includes("=")
      ? argument.split("=", 2)[1]
      : rustToolArguments[profileIndex + 1];
    return profile === "dev" || profile === "test" ? "debug" : profile;
  }
  return rustToolArguments.includes("--release") ? "release" : "debug";
}

function verifyAndStageWindowsTestRuntime(workspaceRoot, runtimeDirectory) {
  const manifest = JSON.parse(readFileSync(join(runtimeDirectory, "manifest.json"), "utf8"));
  const targetSetting = childEnvironment.CARGO_TARGET_DIR;
  const targetRoot = targetSetting
    ? isAbsolute(targetSetting)
      ? targetSetting
      : resolve(process.cwd(), targetSetting)
    : join(workspaceRoot, "target");
  const testExecutableDirectory = join(targetRoot, cargoTestProfileDirectory(), "deps");
  mkdirSync(testExecutableDirectory, { recursive: true });

  for (const fileName of ["DirectML.dll", "onnxruntime.dll", "sherpa-onnx-c-api.dll"]) {
    const source = join(runtimeDirectory, fileName);
    const expectedHash = manifest.files?.[fileName];
    const actualHash = createHash("sha256").update(readFileSync(source)).digest("hex");
    if (!expectedHash || actualHash !== expectedHash) {
      throw new Error(`${fileName} does not match the verified native runtime manifest`);
    }
    copyFileSync(source, join(testExecutableDirectory, fileName));
  }
}

if (process.platform === "win32") {
  const workspaceRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
  const directMlRuntime = join(
    workspaceRoot,
    "apps",
    "desktop",
    "src-tauri",
    "resources",
    "native",
    "sherpa-onnx-1.13.4-directml",
    "windows-x64",
  );
  if (existsSync(join(directMlRuntime, "sherpa-onnx-c-api.lib"))) {
    childEnvironment.SHERPA_ONNX_LIB_DIR = directMlRuntime;
    childEnvironment.PATH = `${directMlRuntime}${delimiter}${childEnvironment.PATH}`;
    if (rustTool === "cargo" && rustToolArguments[0] === "test") {
      // Windows resolves System32 before PATH. Stage the verified pair beside
      // Cargo's test executables so a system onnxruntime.dll cannot be loaded.
      verifyAndStageWindowsTestRuntime(workspaceRoot, directMlRuntime);
    }
  }

  // The sherpa build helper and Tauri's resource build both stage the same
  // runtime DLLs beside the profile binary. Cargo may otherwise run those
  // build scripts concurrently and Windows rejects the second open with
  // ERROR_SHARING_VIOLATION. Quality checks favor deterministic staging over
  // build-script parallelism; explicit caller configuration still wins.
  if (
    rustTool === "cargo" &&
    rustToolArguments[0] === "clippy" &&
    childEnvironment.CARGO_BUILD_JOBS === undefined
  ) {
    childEnvironment.CARGO_BUILD_JOBS = "1";
  }
}

let executable;
let executableArguments;
if (rustTool === "cargo") {
  executable = join(cargoBin, cargoExecutableName);
  executableArguments = rustToolArguments;
} else {
  const currentDirectory = dirname(fileURLToPath(import.meta.url));
  const tauriCli = join(currentDirectory, "..", "node_modules", "@tauri-apps", "cli", "tauri.js");
  if (!existsSync(tauriCli)) {
    console.error("The local Tauri CLI is missing. Run corepack pnpm@11.15.1 install first.");
    process.exit(127);
  }
  executable = process.execPath;
  executableArguments = [tauriCli, ...rustToolArguments];
}

const child = spawn(executable, executableArguments, {
  cwd: process.cwd(),
  env: childEnvironment,
  stdio: "inherit",
  windowsHide: false,
});

child.on("error", (error) => {
  console.error(`Failed to start ${rustTool}: ${error.message}`);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    console.error(`${rustTool} was terminated by signal ${signal}.`);
    process.exit(1);
  }
  process.exit(code ?? 1);
});
