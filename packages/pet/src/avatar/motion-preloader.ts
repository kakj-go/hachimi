import type { MotionCatalogEntry } from "@hachimi/contracts";
import type { MotionAssetLibrary } from "@hachimi/avatar-motion-runtime";
import type { VRM } from "@pixiv/three-vrm";

export class MotionPreloader {
  constructor(private readonly library: MotionAssetLibrary) {}

  async preloadCore(vrm: VRM, entries: readonly MotionCatalogEntry[]): Promise<void> {
    const core = entries.filter(
      (entry) =>
        entry.family === "idle" ||
        entry.family === "speech" ||
        entry.family === "locomotion" ||
        entry.family === "reaction",
    );
    await this.library.preload(core.map((entry) => entry.id));
    await Promise.all(core.slice(0, 12).map((entry) => this.library.prepare(vrm, entry.id)));
  }
}
