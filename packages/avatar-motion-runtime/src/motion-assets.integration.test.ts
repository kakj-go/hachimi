import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import type { MotionCatalogEntry, MotionTransitionProfile } from "@hachimi/contracts";
import { VRMLoaderPlugin, type VRM } from "@pixiv/three-vrm";
import { VRMAnimationLoaderPlugin } from "@pixiv/three-vrm-animation";
import { describe, expect, it } from "vitest";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { MotionAssetLibrary } from "./motion-asset-library";
import type { SampledMotionPose } from "./motion-asset-library";

interface BuiltinCatalog {
  entries: MotionCatalogEntry[];
  transitionProfiles: MotionTransitionProfile[];
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
      await readFile(`${repositoryRoot}/assets/avatar-motions-v5/catalog.json`, "utf8"),
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
    const resolveAsset = async (id: string) => {
      const entry = entries.get(id);
      if (!entry) return null;
      const bytes = await readFile(
        `${repositoryRoot}/assets/avatar-motions-v5/builtin/${entry.fileName}`,
      );
      return {
        entry,
        assetUrl: `data:model/gltf-binary;base64,${bytes.toString("base64")}`,
      };
    };
    const featureCache = new Map<string, string>();
    let featureWrites = 0;
    const cacheAdapter = {
      read: async (key: string) => featureCache.get(key) ?? null,
      write: async (key: string, payload: string) => {
        featureWrites += 1;
        featureCache.set(key, payload);
      },
    };
    const library = new MotionAssetLibrary(motionLoader, resolveAsset, cacheAdapter);
    library.setCatalog(catalog.entries);

    let waitingFingerTracks = 0;
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
      if (entry.name.toLowerCase() === "waiting") {
        waitingFingerTracks = fingerTracks;
      }
      if (!entry.hasFingerMotion) {
        expect(fingerTracks, entry.id).toBe(0);
        noFingerFallbacks += 1;
      }
      expect(library.compiledCount(vrm!), entry.id).toBeLessThanOrEqual(24);
      expect(library.sourceCount(), entry.id).toBeLessThanOrEqual(48);
    }
    expect(waitingFingerTracks).toBeGreaterThanOrEqual(28);
    expect(noFingerFallbacks).toBeGreaterThan(0);

    const derived = catalog.entries.find((entry) => entry.motionRole === "action_recover_to_idle")!;
    const source = entries.get(derived.derivedFromMotionId!)!;
    await library.prepare(vrm!, source.id);
    await library.prepare(vrm!, derived.id);
    const derivedStart = library.sample(vrm!, derived.id, 0)!;
    const sourceStart = library.sample(vrm!, source.id, derived.sourceStartMs ?? 0)!;
    expect(maxPoseAngle(derivedStart, sourceStart)).toBeLessThan(1e-6);
    const derivedEnd = library.sample(vrm!, derived.id, derived.durationMs - 0.001)!;
    const sourceEnd = library.sample(
      vrm!,
      source.id,
      (derived.sourceEndMs ?? source.durationMs) - 0.001,
    )!;
    expect(maxPoseAngle(derivedEnd, sourceEnd)).toBeLessThan(1e-5);

    const profile = catalog.transitionProfiles.find(
      (value) => value.id === source.transitionProfileId,
    )!;
    const firstIndex = await library.prepareFeatureIndex(vrm!, source.id, profile);
    expect(featureWrites).toBe(1);
    const restoredLibrary = new MotionAssetLibrary(motionLoader, resolveAsset, cacheAdapter);
    restoredLibrary.setCatalog(catalog.entries);
    await restoredLibrary.prepare(vrm!, source.id);
    const restoredIndex = await restoredLibrary.prepareFeatureIndex(vrm!, source.id, profile);
    expect(featureWrites).toBe(1);
    expect(restoredIndex.frames.length).toBe(firstIndex.frames.length);
  }, 120_000);
});

function maxPoseAngle(left: SampledMotionPose, right: SampledMotionPose): number {
  let maximum = 0;
  for (const [name, rotation] of left.rotations) {
    const other = right.rotations.get(name);
    if (other) maximum = Math.max(maximum, rotation.angleTo(other));
  }
  return maximum;
}

function exactArrayBuffer(buffer: Buffer): ArrayBuffer {
  return buffer.buffer.slice(
    buffer.byteOffset,
    buffer.byteOffset + buffer.byteLength,
  ) as ArrayBuffer;
}
