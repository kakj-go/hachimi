import { VRMUtils } from "@pixiv/three-vrm";
import type { Object3D } from "three";
import type { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";

/**
 * WebView2 may expose createImageBitmap while failing to upload bitmap-backed textures on a
 * transparent WebGL surface. Force GLTFLoader to use its HTMLImageElement path while parsing.
 */
export async function loadAvatarWithDomTextures(loader: GLTFLoader, assetUrl: string) {
  const response = await fetch(assetUrl, { cache: "no-store" });
  if (!response.ok) throw new Error(`Unable to load avatar (HTTP ${response.status})`);
  const bytes = await response.arrayBuffer();
  const bitmapHost = globalThis as unknown as Record<string, unknown>;
  const originalCreateImageBitmap = bitmapHost["createImageBitmap"];
  let parsePromise: ReturnType<GLTFLoader["parseAsync"]>;
  try {
    bitmapHost["createImageBitmap"] = undefined;
    parsePromise = loader.parseAsync(bytes, "");
  } finally {
    bitmapHost["createImageBitmap"] = originalCreateImageBitmap;
  }
  return parsePromise;
}

export function deepDisposeAvatarRoot(root: Object3D): void {
  VRMUtils.deepDispose(root);
}
