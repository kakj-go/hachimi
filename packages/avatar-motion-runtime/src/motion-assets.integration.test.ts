import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import type { MotionCatalogEntry } from "@hachimi/contracts";
import { VRMLoaderPlugin, type VRM } from "@pixiv/three-vrm";
import { VRMAnimationLoaderPlugin } from "@pixiv/three-vrm-animation";
import { describe, expect, it } from "vitest";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { MotionAssetLibrary } from "./motion-asset-library";

interface BuiltinCatalog {
  entries: MotionCatalogEntry[];
}

const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url));

Object.defineProperty(globalThis, "self", { configurable: true, value: globalThis });
Object.defineProperty(globalThis, "createImageBitmap", {
  configurable: true,
  value: async () => ({ close() {}, height: 1, width: 1 }) as ImageBitmap,
});

describe("bundled VRMA runtime integration", () => {
  it("retargets and samples every built-in motion on the default VRM", async () => {
    const catalog = JSON.parse(
      await readFile(`${repositoryRoot}/assets/avatar-motions-v4/catalog.json`, "utf8"),
    ) as BuiltinCatalog;
    const avatarLoader = new GLTFLoader();
    avatarLoader.register((parser) => new VRMLoaderPlugin(parser));
    const avatarBytes = await readFile(
      `${repositoryRoot}/assets/avatar-default/2639776812528692620/2639776812528692620.vrm`,
    );
    const avatarGltf = await avatarLoader.parseAsync(exactArrayBuffer(avatarBytes), "");
    const vrm = avatarGltf.userData["vrm"] as VRM | undefined;
    expect(vrm).toBeDefined();

    const entries = new Map(catalog.entries.map((entry) => [entry.id, entry]));
    const motionLoader = new GLTFLoader();
    motionLoader.register((parser) => new VRMAnimationLoaderPlugin(parser));
    const library = new MotionAssetLibrary(motionLoader, async (id) => {
      const entry = entries.get(id);
      if (!entry) return null;
      const bytes = await readFile(
        `${repositoryRoot}/assets/avatar-motions-v4/builtin/${entry.fileName}`,
      );
      return {
        entry,
        assetUrl: `data:model/gltf-binary;base64,${bytes.toString("base64")}`,
      };
    });
    library.setCatalog(catalog.entries);

    let standardWaitingFingerTracks = 0;
    let noFingerFallbacks = 0;
    for (const entry of catalog.entries) {
      await library.prepare(vrm!, entry.id);
      for (const timeMs of [0, entry.durationMs * 0.5, Math.max(entry.durationMs - 0.001, 0)]) {
        const sample = library.sample(vrm!, entry.id, timeMs);
        expect(sample, entry.id).toBeDefined();
        for (const rotation of sample!.rotations.values()) {
          expect(rotation.toArray().every(Number.isFinite), entry.id).toBe(true);
          expect(rotation.lengthSq(), entry.id).toBeCloseTo(1, 4);
        }
        expect(sample!.hipsPosition?.toArray().every(Number.isFinite) ?? true, entry.id).toBe(true);
      }
      const middle = library.sample(vrm!, entry.id, entry.durationMs * 0.5)!;
      const fingerTracks = [...middle.rotations.keys()].filter((bone) =>
        /_(thumb|index|middle|ring|little)_/.test(bone),
      ).length;
      if (entry.name.toLowerCase() === "standard waiting") {
        standardWaitingFingerTracks = fingerTracks;
      }
      if (!entry.hasFingerMotion) {
        expect(fingerTracks, entry.id).toBe(0);
        noFingerFallbacks += 1;
      }
      expect(library.compiledCount(vrm!), entry.id).toBeLessThanOrEqual(24);
      expect(library.sourceCount(), entry.id).toBeLessThanOrEqual(48);
    }
    expect(standardWaitingFingerTracks).toBeGreaterThanOrEqual(28);
    expect(noFingerFallbacks).toBeGreaterThan(0);
  }, 120_000);
});

function exactArrayBuffer(buffer: Buffer): ArrayBuffer {
  return buffer.buffer.slice(
    buffer.byteOffset,
    buffer.byteOffset + buffer.byteLength,
  ) as ArrayBuffer;
}
