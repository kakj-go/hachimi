import { deepDisposeAvatarRoot, loadAvatarWithDomTextures } from "@hachimi/avatar-motion-runtime";
import { commandFailure, commands, type AvatarRuntimeAsset } from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { VRMLoaderPlugin, VRMUtils, type VRM } from "@pixiv/three-vrm";
import { Show, createSignal, onCleanup, onMount } from "solid-js";
import {
  AmbientLight,
  Box3,
  DirectionalLight,
  Group,
  HemisphereLight,
  MathUtils,
  NeutralToneMapping,
  PerspectiveCamera,
  Scene,
  SRGBColorSpace,
  Vector3,
  WebGLRenderer,
} from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";

export function AvatarCardPreview(props: { entryId: string; name: string }) {
  const i18n = useI18n();
  const text = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const [loading, setLoading] = createSignal(true);
  const [failure, setFailure] = createSignal<string>();
  let host: HTMLDivElement | undefined;
  let runtime: AvatarCardPreviewRuntime | undefined;
  let disposed = false;

  onMount(() => {
    if (!host) return;
    runtime = new AvatarCardPreviewRuntime(host);
    void commands
      .getAvatarRuntimeAsset(props.entryId)
      .then(async (asset) => {
        if (!asset) throw new Error(text("模型不可用", "Avatar is unavailable"));
        await runtime?.load(asset);
        if (!disposed) setLoading(false);
      })
      .catch((error) => {
        if (!disposed) {
          setLoading(false);
          setFailure(commandFailure(error).message);
        }
      });
  });

  onCleanup(() => {
    disposed = true;
    runtime?.dispose();
  });

  return (
    <div
      class="avatar-card-preview"
      ref={host}
      role="img"
      aria-label={`${props.name} · ${text("模型预览", "avatar preview")}`}
    >
      <Show when={loading()}>
        <span class="avatar-card-preview-status">{text("加载预览…", "Loading preview…")}</span>
      </Show>
      <Show when={failure()}>
        <span class="avatar-card-preview-status" title={failure()}>
          {text("预览不可用", "Preview unavailable")}
        </span>
      </Show>
    </div>
  );
}

class AvatarCardPreviewRuntime {
  private readonly renderer: WebGLRenderer;
  private readonly scene = new Scene();
  private readonly camera = new PerspectiveCamera(28, 1, 0.01, 100);
  private readonly loader = new GLTFLoader();
  private readonly resizeObserver: ResizeObserver;
  private root: Group | undefined;
  private loadGeneration = 0;
  private disposed = false;

  constructor(private readonly container: HTMLElement) {
    this.renderer = new WebGLRenderer({ antialias: true, alpha: true, premultipliedAlpha: true });
    this.renderer.outputColorSpace = SRGBColorSpace;
    this.renderer.toneMapping = NeutralToneMapping;
    this.renderer.toneMappingExposure = 0.92;
    this.renderer.setPixelRatio(Math.min(devicePixelRatio, 1.5));
    this.renderer.domElement.setAttribute("aria-hidden", "true");
    this.container.prepend(this.renderer.domElement);
    this.loader.register((parser) => new VRMLoaderPlugin(parser));
    this.scene.add(new AmbientLight(0xffffff, 0.8));
    this.scene.add(new HemisphereLight(0xf2f0ff, 0x554b68, 0.9));
    const key = new DirectionalLight(0xfff7f1, 1.7);
    key.position.set(3, 5, 5);
    this.scene.add(key);
    const fill = new DirectionalLight(0xb9ccff, 0.5);
    fill.position.set(-4, 2, 3);
    this.scene.add(fill);
    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(container);
    this.resize();
  }

  async load(asset: AvatarRuntimeAsset): Promise<void> {
    const generation = ++this.loadGeneration;
    const gltf = await loadAvatarWithDomTextures(this.loader, asset.assetUrl);
    if (this.disposed || generation !== this.loadGeneration) {
      deepDisposeAvatarRoot(gltf.scene);
      return;
    }
    const vrm = gltf.userData["vrm"] as VRM | undefined;
    if (!vrm) {
      deepDisposeAvatarRoot(gltf.scene);
      throw new Error("Avatar preview requires a Runtime Ready VRM");
    }
    if (asset.format === "vrm0") VRMUtils.rotateVRM0(vrm);
    const root = new Group();
    root.add(vrm.scene);
    const bounds = new Box3().setFromObject(root);
    const center = bounds.getCenter(new Vector3());
    vrm.scene.position.set(-center.x, -bounds.min.y, -center.z);
    this.root = root;
    this.scene.add(root);
    vrm.update(0);
    this.frameAvatar(root);
    this.render();
  }

  dispose(): void {
    this.disposed = true;
    this.loadGeneration += 1;
    this.resizeObserver.disconnect();
    if (this.root) deepDisposeAvatarRoot(this.root);
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }

  private resize(): void {
    const width = Math.max(this.container.clientWidth, 1);
    const height = Math.max(this.container.clientHeight, 1);
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.render();
  }

  private frameAvatar(root: Group): void {
    const size = new Box3().setFromObject(root).getSize(new Vector3());
    const targetY = size.y * 0.5;
    const distance = (size.y / (2 * Math.tan(MathUtils.degToRad(this.camera.fov / 2)))) * 1.15;
    this.camera.position.set(0, targetY, distance);
    this.camera.lookAt(0, targetY, 0);
    this.camera.updateProjectionMatrix();
  }

  private render(): void {
    if (!this.disposed) this.renderer.render(this.scene, this.camera);
  }
}
