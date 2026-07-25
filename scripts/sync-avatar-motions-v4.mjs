import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(SCRIPT_DIR, "..");
const OUTPUT_ROOT = join(ROOT, "assets", "avatar-motions-v4");
const BUILTIN_ROOT = join(OUTPUT_ROOT, "builtin");
const NOTICE_ROOT = join(OUTPUT_ROOT, "notices");
const CATALOG_PATH = join(OUTPUT_ROOT, "catalog.json");

const SOURCES = [
  {
    id: "clawatar",
    label: "Clawatar",
    repository: "https://github.com/Dongping-Chen/Clawatar.git",
    commit: "e7c40f1a7b4526c854d5219fbd18225f9504e10f",
    paths: ["public/animations"],
  },
  {
    id: "openmaiwaifu",
    label: "OpenMaiWaifu",
    repository: "https://github.com/buyve/OpenMaiWaifu.git",
    commit: "fb2fbc1a1d56c6f63a8d2df367b978b2be7a4580",
    paths: ["assets/motions", "public/motions"],
  },
];

// Product exclusions: these clips are intentionally absent from the built-in catalog and bundle.
const EXCLUDED_SOURCE_PATHS = new Set([
  "clawatar/public/animations/75_Sitting.vrma",
  "clawatar/public/animations/76_Sitting_2.vrma",
  "clawatar/public/animations/149_Sitting Idle.vrma",
  "clawatar/public/animations/150_Sitting Laughing.vrma",
  "openmaiwaifu/assets/motions/test-pixiv.vrma",
  "openmaiwaifu/assets/motions/walk.vrma",
  "openmaiwaifu/public/motions/photobooth-greeting.vrma",
]);

const FINGER_PATTERN = /(Thumb|Index|Middle|Ring|Little)/;
const UPPER_BODY = new Set([
  "spine",
  "chest",
  "upperChest",
  "neck",
  "head",
  "leftShoulder",
  "leftUpperArm",
  "leftLowerArm",
  "leftHand",
  "rightShoulder",
  "rightUpperArm",
  "rightLowerArm",
  "rightHand",
]);

const MOTION_NAME_ZH = new Map(
  Object.entries({
    Angry: "生气",
    "Angry Gesture": "生气手势",
    "Annoyed Head Shake": "烦恼摇头",
    appearing: "登场",
    "Arms Hip Hop Dance": "手臂嘻哈舞",
    "Bboy Hip Hop Move": "B-boy 嘻哈动作",
    "Belly Dance": "肚皮舞",
    Bellydancing: "肚皮舞表演",
    "Chicken Dance": "小鸡舞",
    Clapping: "鼓掌",
    "cool liked": "酷系开心回应",
    "cool waiting": "酷系待机",
    Crying: "哭泣",
    "Dancing Twerk": "电臀舞",
    Defeat: "失败",
    "Drunk Idle": "醉酒待机",
    "Drunk Idle Variation": "醉酒待机变化",
    "energetic liked": "活力开心回应",
    "energetic waiting": "活力待机",
    "flamboyant liked": "华丽开心回应",
    "flamboyant waiting": "华丽待机",
    "gentleman liked": "绅士开心回应",
    "gentleman waiting": "绅士待机",
    happy: "开心",
    "Happy Hand Gesture": "开心手势",
    "Head Nod Yes": "点头同意",
    "Hip Hop Dancing": "嘻哈舞",
    "Hip Hop Dancing 2": "嘻哈舞 2",
    "Hip Hop Dancing 3": "嘻哈舞 3",
    "Hip Hop Dancing 4": "嘻哈舞 4",
    "House Dancing": "浩室舞",
    "House Dancing 2": "浩室舞 2",
    Idle: "待机",
    "Jazz Dancing": "爵士舞",
    "Joyful Jump": "开心跳跃",
    "ladylike liked": "淑女开心回应",
    "ladylike waiting": "淑女待机",
    laughing: "大笑",
    Leaning: "倚靠",
    liked: "开心回应",
    Loser: "失败手势",
    "Macarena Dance": "玛卡莲娜舞",
    "manly appearing": "帅气登场",
    "Neck Stretching": "颈部拉伸",
    No: "拒绝",
    "photobooth greeting": "拍照亭问候",
    "photobooth model pose": "拍照亭模特姿势",
    "photobooth peace sign": "拍照亭胜利手势",
    "photobooth shoot": "拍照亭拍摄姿势",
    "photobooth show full body": "拍照亭全身展示",
    "photobooth spin": "拍照亭旋转",
    "photobooth squat": "拍照亭下蹲",
    "powerful liked": "力量型开心回应",
    "powerful waiting": "力量型待机",
    "Quick Formal Bow": "快速正式鞠躬",
    "Quick Informal Bow": "快速随意鞠躬",
    Rejected: "被拒绝",
    "Relieved Sigh": "放松叹气",
    "reze dance hard": "蕾塞高强度舞蹈",
    "Rumba Dancing": "伦巴舞",
    "Sad Idle": "伤心待机",
    "Shaking Head No": "摇头拒绝",
    Shrugging: "耸肩",
    "shy liked": "害羞开心回应",
    "shy waiting": "害羞待机",
    "Silly Dancing": "搞怪舞蹈",
    Singing: "唱歌",
    Sitting: "坐姿",
    "Sitting 2": "坐姿 2",
    "Sitting Idle": "坐姿待机",
    "Sitting Laughing": "坐姿大笑",
    "Snake Hip Hop Dance": "蛇形嘻哈舞",
    "standard liked": "标准开心回应",
    "standard waiting": "标准待机",
    "Standing Greeting": "站立问候",
    stretching: "伸展",
    "Swing Dancing": "摇摆舞",
    "Swing Dancing 2": "摇摆舞 2",
    "Swing Dancing 3": "摇摆舞 3",
    Talking: "说话",
    "Talking On Phone": "打电话",
    "test pixiv": "Pixiv 测试动作",
    Thankful: "感谢",
    Thinking: "思考",
    waiting: "等待",
    walk: "行走",
    Waving: "挥手",
    "Whatever Gesture": "随意手势",
  }),
);

const checkOnly = process.argv.includes("--check");
if (checkOnly) {
  checkCatalog();
} else {
  syncCatalog();
}

function syncCatalog() {
  const temporaryRoot = mkdtempSync(join(tmpdir(), "hachimi-motion-v4-"));
  const records = [];
  try {
    for (const source of SOURCES) {
      const checkout = join(temporaryRoot, source.id);
      clonePinnedSource(source, checkout);
      for (const sourcePath of source.paths) {
        const absolute = join(checkout, sourcePath);
        if (!existsSync(absolute)) continue;
        for (const path of walkFiles(absolute).filter(
          (value) => extname(value).toLowerCase() === ".vrma",
        )) {
          const normalizedSourcePath = `${source.id}/${relative(checkout, path).replaceAll("\\", "/")}`;
          if (EXCLUDED_SOURCE_PATHS.has(normalizedSourcePath)) continue;
          records.push(readMotionRecord(source, checkout, path));
        }
      }
    }

    const grouped = new Map();
    for (const record of records.sort((left, right) =>
      left.sourcePath.localeCompare(right.sourcePath),
    )) {
      const existing = grouped.get(record.sha256);
      if (existing) {
        existing.sourcePaths.push(record.sourcePath);
        if (!existing.sourceProjects.includes(record.sourceProject)) {
          existing.sourceProjects.push(record.sourceProject);
        }
        continue;
      }
      grouped.set(record.sha256, {
        ...record,
        sourcePaths: [record.sourcePath],
        sourceProjects: [record.sourceProject],
      });
    }

    rmSync(OUTPUT_ROOT, { recursive: true, force: true });
    mkdirSync(BUILTIN_ROOT, { recursive: true });
    mkdirSync(NOTICE_ROOT, { recursive: true });

    const entries = [...grouped.values()]
      .map((record) => materializeEntry(record))
      .sort((left, right) => left.id.localeCompare(right.id));
    const catalog = {
      schemaVersion: 1,
      specVersion: "1.0",
      generatedAt: new Date().toISOString(),
      sources: SOURCES.map(({ id, label, repository, commit }) => ({
        id,
        label,
        repository: repository.replace(/\.git$/, ""),
        commit,
      })),
      entries,
    };
    writeFileSync(CATALOG_PATH, `${JSON.stringify(catalog, null, 2)}\n`);
    writeNotices();
    checkCatalog();
    process.stdout.write(
      `Avatar Motion V4: ${records.length} source files, ${entries.length} unique VRMA assets.\n`,
    );
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

function clonePinnedSource(source, destination) {
  execFileSync(
    "git",
    ["clone", "--quiet", "--filter=blob:none", "--no-checkout", source.repository, destination],
    {
      stdio: "inherit",
    },
  );
  execFileSync(
    "git",
    ["-C", destination, "sparse-checkout", "set", ...source.paths, "LICENSE", "README.md"],
    {
      stdio: "inherit",
    },
  );
  execFileSync("git", ["-C", destination, "checkout", "--quiet", source.commit], {
    stdio: "inherit",
  });
}

function readMotionRecord(source, checkout, path) {
  let bytes = readFileSync(path);
  let gltf = parseGlb(bytes, path);
  const extension = gltf.json.extensions?.VRMC_vrm_animation;
  if (extension?.specVersion !== "1.0") {
    throw new Error(`${path}: expected VRMC_vrm_animation specVersion 1.0`);
  }
  if (!Array.isArray(gltf.json.animations) || gltf.json.animations.length !== 1) {
    throw new Error(`${path}: expected exactly one glTF animation`);
  }

  let humanBones = extension.humanoid?.humanBones ?? {};
  let repairedHumanoidMapping = false;
  if (Object.keys(humanBones).length === 0) {
    humanBones = inferHumanoidMapping(gltf.json.nodes ?? []);
    if (Object.keys(humanBones).length === 0) {
      throw new Error(`${path}: empty humanoid mapping cannot be repaired`);
    }
    extension.humanoid = { ...(extension.humanoid ?? {}), humanBones };
    bytes = encodeGlb(gltf.json, gltf.bin);
    gltf = parseGlb(bytes, path);
    repairedHumanoidMapping = true;
  }
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  const nodeToBone = new Map(
    Object.entries(humanBones)
      .filter(([, value]) => Number.isInteger(value?.node))
      .map(([bone, value]) => [value.node, bone]),
  );
  const animation = gltf.json.animations[0];
  validateAnimationAccessors(gltf, animation, path);
  const animatedBones = [
    ...new Set(
      animation.channels.map((channel) => nodeToBone.get(channel.target?.node)).filter(Boolean),
    ),
  ].sort();
  if (animatedBones.length === 0) throw new Error(`${path}: no animated humanoid bone tracks`);
  const scaleTracks = animation.channels.filter(
    (channel) => channel.target?.path === "scale",
  ).length;
  const ignoredTranslations = animation.channels.filter(
    (channel) =>
      channel.target?.path === "translation" && nodeToBone.get(channel.target?.node) !== "hips",
  ).length;
  const durationMs = Math.max(1, Math.round(readAnimationDuration(gltf, animation) * 1000));
  const stem = basename(path, extname(path));
  const category = inferCategory(stem);
  const title = cleanTitle(stem);
  const sourcePath = `${source.id}/${relative(checkout, path).replaceAll("\\", "/")}`;
  return {
    sourceId: source.id,
    sourceProject: source.label,
    sourcePath,
    bytes,
    sha256,
    sizeBytes: bytes.length,
    durationMs,
    title,
    category,
    tags: inferTags(stem, category),
    playbackMode: inferPlaybackMode(stem, category),
    rootMode: inferRootMode(stem, category),
    channels: inferChannels(animatedBones),
    animatedBones,
    fingerBoneCount: animatedBones.filter((bone) => FINGER_PATTERN.test(bone)).length,
    hasExpression: Boolean(extension.expressions),
    hasLookAt: Boolean(extension.lookAt),
    mirrorable: inferMirrorable(stem),
    transitionInMs: category === "idle" ? 350 : 220,
    transitionOutMs: category === "idle" ? 350 : 260,
    warnings: [
      ...(repairedHumanoidMapping ? ["repaired_empty_humanoid_mapping"] : []),
      ...(scaleTracks > 0 ? [`ignored_scale_tracks:${scaleTracks}`] : []),
      ...(ignoredTranslations > 0 ? [`ignored_non_hips_translations:${ignoredTranslations}`] : []),
    ],
  };
}

function materializeEntry(record) {
  const slug = slugify(record.title);
  const id = `builtin.${record.sourceId}.${slug}.${record.sha256.slice(0, 8)}`;
  const fileName = `${record.sha256}.vrma`;
  writeFileSync(join(BUILTIN_ROOT, fileName), record.bytes);
  const nameZh = localizedMotionName(record.title);
  const description = motionDescription(record.title, record.category);
  const descriptionZh = motionDescriptionZh(nameZh, record.category);
  return {
    id,
    source: "builtin",
    protected: true,
    name: record.title,
    nameZh,
    description,
    descriptionZh,
    fileName,
    sha256: record.sha256,
    sizeBytes: record.sizeBytes,
    durationMs: record.durationMs,
    category: record.category,
    tags: record.tags,
    playbackMode: record.playbackMode,
    rootMode: record.rootMode,
    channels: record.channels,
    animatedBones: record.animatedBones,
    fingerBoneCount: record.fingerBoneCount,
    hasFingerMotion: record.fingerBoneCount > 0,
    hasExpression: record.hasExpression,
    hasLookAt: record.hasLookAt,
    mirrorable: record.mirrorable,
    transitionInMs: record.transitionInMs,
    transitionOutMs: record.transitionOutMs,
    sourceProject: record.sourceProjects.join(" / "),
    sourcePaths: record.sourcePaths,
    warnings: record.warnings,
  };
}

function localizedMotionName(title) {
  const dance = /^Dance Motion (\d+)$/i.exec(title);
  if (dance) return `舞蹈动作 ${dance[1]}`;
  const translated = MOTION_NAME_ZH.get(title);
  if (!translated) throw new Error(`missing Chinese motion name: ${title}`);
  return translated;
}

function motionDescription(title, category) {
  return {
    idle: `A looping “${title}” motion for natural ambient avatar behavior.`,
    reaction: `A “${title}” reaction for touch and interaction feedback.`,
    gesture: `A “${title}” gesture for greetings, conversation, or interaction feedback.`,
    speech: `A “${title}” body motion for natural speech accompaniment.`,
    locomotion: `A “${title}” locomotion clip for bounded stage movement.`,
    performance: `A full-body “${title}” performance motion.`,
  }[category];
}

function motionDescriptionZh(name, category) {
  return {
    idle: `用于自然环境行为的循环“${name}”动作。`,
    reaction: `用于触摸和互动反馈的“${name}”反应动作。`,
    gesture: `适用于问候、对话或互动反馈的“${name}”手势。`,
    speech: `用于自然配合语音输出的“${name}”身体动作。`,
    locomotion: `用于有限舞台移动的“${name}”行走动作。`,
    performance: `完整的全身“${name}”表演动作。`,
  }[category];
}

function parseGlb(bytes, path) {
  if (
    bytes.length < 20 ||
    bytes.toString("ascii", 0, 4) !== "glTF" ||
    bytes.readUInt32LE(4) !== 2
  ) {
    throw new Error(`${path}: not a glTF 2.0 binary`);
  }
  let offset = 12;
  let json;
  let bin = Buffer.alloc(0);
  while (offset + 8 <= bytes.length) {
    const length = bytes.readUInt32LE(offset);
    const type = bytes.readUInt32LE(offset + 4);
    const data = bytes.subarray(offset + 8, offset + 8 + length);
    if (type === 0x4e4f534a) json = JSON.parse(data.toString("utf8").trimEnd());
    if (type === 0x004e4942) bin = data;
    offset += 8 + length;
  }
  if (!json) throw new Error(`${path}: missing JSON chunk`);
  return { json, bin };
}

function encodeGlb(json, bin) {
  const rawJson = Buffer.from(JSON.stringify(json), "utf8");
  const jsonLength = Math.ceil(rawJson.length / 4) * 4;
  const jsonChunk = Buffer.alloc(jsonLength, 0x20);
  rawJson.copy(jsonChunk);
  const binLength = Math.ceil(bin.length / 4) * 4;
  const binChunk = Buffer.alloc(binLength);
  bin.copy(binChunk);
  const totalLength = 12 + 8 + jsonChunk.length + (binChunk.length > 0 ? 8 + binChunk.length : 0);
  const output = Buffer.alloc(totalLength);
  output.write("glTF", 0, "ascii");
  output.writeUInt32LE(2, 4);
  output.writeUInt32LE(totalLength, 8);
  output.writeUInt32LE(jsonChunk.length, 12);
  output.writeUInt32LE(0x4e4f534a, 16);
  jsonChunk.copy(output, 20);
  if (binChunk.length > 0) {
    const offset = 20 + jsonChunk.length;
    output.writeUInt32LE(binChunk.length, offset);
    output.writeUInt32LE(0x004e4942, offset + 4);
    binChunk.copy(output, offset + 8);
  }
  return output;
}

function inferHumanoidMapping(nodes) {
  const aliases = {
    hips: "hips",
    spine: "spine",
    spine1: "chest",
    spine2: "upperChest",
    neck: "neck",
    head: "head",
    leftshoulder: "leftShoulder",
    leftarm: "leftUpperArm",
    leftforearm: "leftLowerArm",
    lefthand: "leftHand",
    rightshoulder: "rightShoulder",
    rightarm: "rightUpperArm",
    rightforearm: "rightLowerArm",
    righthand: "rightHand",
    leftupleg: "leftUpperLeg",
    leftleg: "leftLowerLeg",
    leftfoot: "leftFoot",
    lefttoebase: "leftToes",
    rightupleg: "rightUpperLeg",
    rightleg: "rightLowerLeg",
    rightfoot: "rightFoot",
    righttoebase: "rightToes",
    lefthandthumb1: "leftThumbMetacarpal",
    lefthandthumb2: "leftThumbProximal",
    lefthandthumb3: "leftThumbDistal",
    lefthandindex1: "leftIndexProximal",
    lefthandindex2: "leftIndexIntermediate",
    lefthandindex3: "leftIndexDistal",
    righthandthumb1: "rightThumbMetacarpal",
    righthandthumb2: "rightThumbProximal",
    righthandthumb3: "rightThumbDistal",
    righthandindex1: "rightIndexProximal",
    righthandindex2: "rightIndexIntermediate",
    righthandindex3: "rightIndexDistal",
  };
  const result = {};
  nodes.forEach((node, nodeIndex) => {
    const normalized = String(node.name ?? "")
      .replace(/^mixamorig:?/i, "")
      .replaceAll(/[^a-z0-9]/gi, "")
      .toLowerCase();
    const bone = aliases[normalized];
    if (bone && !result[bone]) result[bone] = { node: nodeIndex };
  });
  return result;
}

function readAnimationDuration(gltf, animation) {
  let duration = 0;
  for (const sampler of animation.samplers ?? []) {
    const accessor = gltf.json.accessors?.[sampler.input];
    if (!accessor || accessor.type !== "SCALAR" || accessor.componentType !== 5126) continue;
    if (Array.isArray(accessor.max) && Number.isFinite(accessor.max[0])) {
      duration = Math.max(duration, accessor.max[0]);
      continue;
    }
    const view = gltf.json.bufferViews?.[accessor.bufferView];
    if (!view || view.buffer !== 0) continue;
    const start = (view.byteOffset ?? 0) + (accessor.byteOffset ?? 0);
    const stride = view.byteStride ?? 4;
    for (let index = 0; index < accessor.count; index += 1) {
      duration = Math.max(duration, gltf.bin.readFloatLE(start + index * stride));
    }
  }
  if (!Number.isFinite(duration) || duration <= 0)
    throw new Error("animation has no positive duration");
  return duration;
}

function validateAnimationAccessors(gltf, animation, label) {
  for (const [samplerIndex, sampler] of (animation.samplers ?? []).entries()) {
    const times = readAccessorRows(gltf, sampler.input, label).map((row) => row[0]);
    if (
      times.length === 0 ||
      times.some(
        (value, index) =>
          !Number.isFinite(value) || value < 0 || (index > 0 && value < times[index - 1]),
      )
    ) {
      throw new Error(`${label}: sampler ${samplerIndex} has an invalid time sequence`);
    }
    readAccessorRows(gltf, sampler.output, label);
  }
  for (const channel of animation.channels ?? []) {
    if (channel.target?.path !== "rotation") continue;
    const sampler = animation.samplers?.[channel.sampler];
    if (!sampler) throw new Error(`${label}: rotation channel has no sampler`);
    const rows = readAccessorRows(gltf, sampler.output, label);
    const values =
      sampler.interpolation === "CUBICSPLINE" ? rows.filter((_, index) => index % 3 === 1) : rows;
    if (
      values.some(
        (row) =>
          row.length !== 4 || Math.hypot(row[0] ?? 0, row[1] ?? 0, row[2] ?? 0, row[3] ?? 0) < 1e-5,
      )
    ) {
      throw new Error(`${label}: rotation channel contains an invalid quaternion`);
    }
  }
}

function readAccessorRows(gltf, accessorIndex, label) {
  const accessor = gltf.json.accessors?.[accessorIndex];
  const view = gltf.json.bufferViews?.[accessor?.bufferView];
  const componentCounts = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4 };
  const components = componentCounts[accessor?.type];
  if (!accessor || !view || view.buffer !== 0 || accessor.componentType !== 5126 || !components) {
    throw new Error(`${label}: animation accessor ${accessorIndex} is not float buffer data`);
  }
  const elementBytes = components * 4;
  const stride = view.byteStride ?? elementBytes;
  const start = (view.byteOffset ?? 0) + (accessor.byteOffset ?? 0);
  const rows = [];
  for (let index = 0; index < accessor.count; index += 1) {
    const offset = start + index * stride;
    const row = Array.from({ length: components }, (_, component) =>
      gltf.bin.readFloatLE(offset + component * 4),
    );
    if (row.some((value) => !Number.isFinite(value))) {
      throw new Error(`${label}: animation accessor ${accessorIndex} contains NaN/Infinity`);
    }
    rows.push(row);
  }
  return rows;
}

function inferCategory(stem) {
  const value = stem.toLowerCase();
  if (/(idle|waiting|relax|leaning|sleepy|lookaround|look-around)/.test(value)) return "idle";
  if (/(walk|run|situp to idle)/.test(value)) return "locomotion";
  if (/(talk|sing|phone)/.test(value)) return "speech";
  if (/(dance|dancing|spin|jump|squat|model.pose|show.full.body|shoot)/.test(value))
    return "performance";
  if (/(angry|sad|cry|surpris|reject|defeat|annoy|relieved|blush)/.test(value)) return "reaction";
  return "gesture";
}

function inferPlaybackMode(stem, category) {
  if (category === "idle" || /(walk|run|dance|dancing)/i.test(stem)) return "loop";
  return "once";
}

function inferRootMode(stem, category) {
  if (category === "locomotion") return "stage";
  if (category === "performance" && /(dance|dancing|spin|jump)/i.test(stem)) return "stage";
  return "in_place";
}

function inferTags(stem, category) {
  const value = stem.toLowerCase();
  const tags = new Set([category]);
  for (const [pattern, tag] of [
    [/happy|joy|cheer|laugh/, "happy"],
    [/sad|cry|defeat|reject/, "sad"],
    [/angry|annoy/, "angry"],
    [/wave|greet|goodbye/, "wave"],
    [/think/, "thinking"],
    [/nod|yes/, "nod"],
    [/no|shake/, "shake"],
    [/clap/, "clap"],
    [/bow|thank/, "polite"],
    [/shy|blush|innocent/, "shy"],
  ]) {
    if (pattern.test(value)) tags.add(tag);
  }
  return [...tags];
}

function inferChannels(bones) {
  const set = new Set(bones);
  const hasLower = [...set].some((bone) => /(UpperLeg|LowerLeg|Foot|Toes)|hips/.test(bone));
  const hasLeft = [...set].some(
    (bone) => bone.startsWith("left") && /(Arm|Hand|Thumb|Index|Middle|Ring|Little)/.test(bone),
  );
  const hasRight = [...set].some(
    (bone) => bone.startsWith("right") && /(Arm|Hand|Thumb|Index|Middle|Ring|Little)/.test(bone),
  );
  if (hasLower) return ["full_body"];
  const channels = ["upper_body"];
  if (hasLeft) channels.push("left_arm");
  if (hasRight) channels.push("right_arm");
  if ([...set].some((bone) => FINGER_PATTERN.test(bone))) channels.push("fingers");
  if ([...set].some((bone) => !UPPER_BODY.has(bone) && !FINGER_PATTERN.test(bone)))
    channels.push("full_body");
  return [...new Set(channels)];
}

function inferMirrorable(stem) {
  return !/(left|right|phone|peace|shoot|salute)/i.test(stem);
}

function cleanTitle(stem) {
  return stem
    .replace(/^\d+_/, "")
    .replace(/^dm_\d+$/i, (value) => `Dance Motion ${value.slice(3)}`)
    .replaceAll(/[-_]+/g, " ")
    .replaceAll(/\s+/g, " ")
    .trim();
}

function slugify(value) {
  const slug = value
    .toLowerCase()
    .replaceAll(/[^a-z0-9]+/g, "-")
    .replaceAll(/^-|-$/g, "");
  return slug || "motion";
}

function walkFiles(root) {
  const result = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) result.push(...walkFiles(path));
    else if (entry.isFile()) result.push(path);
  }
  return result;
}

function writeNotices() {
  const lines = [
    "# Avatar Motion V4 sources",
    "",
    "The bundled files are pinned, content-addressed copies from the following upstream repositories:",
    "",
    ...SOURCES.map(
      (source) =>
        `- ${source.label}: ${source.repository.replace(/\.git$/, "")} @ \`${source.commit}\``,
    ),
    "",
    "See the copied upstream license files in this directory and each catalog entry's sourcePaths.",
    "",
  ];
  writeFileSync(join(NOTICE_ROOT, "README.md"), lines.join("\n"));
  const temporaryRoot = mkdtempSync(join(tmpdir(), "hachimi-motion-notice-"));
  try {
    for (const source of SOURCES) {
      const checkout = join(temporaryRoot, source.id);
      clonePinnedSource(source, checkout);
      const license = ["LICENSE", "LICENSE.md", "LICENSE.txt"]
        .map((name) => join(checkout, name))
        .find(existsSync);
      if (license)
        cpSync(license, join(NOTICE_ROOT, `${source.id}-LICENSE${extname(license) || ".txt"}`));
    }
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

function checkCatalog() {
  if (!existsSync(CATALOG_PATH)) throw new Error(`missing ${CATALOG_PATH}`);
  const catalog = JSON.parse(readFileSync(CATALOG_PATH, "utf8"));
  if (
    catalog.schemaVersion !== 1 ||
    catalog.specVersion !== "1.0" ||
    !Array.isArray(catalog.entries)
  ) {
    throw new Error("invalid Avatar Motion V4 catalog header");
  }
  const ids = new Set();
  const hashes = new Set();
  for (const entry of catalog.entries) {
    if (ids.has(entry.id)) throw new Error(`duplicate motion id: ${entry.id}`);
    if (hashes.has(entry.sha256)) throw new Error(`duplicate motion blob: ${entry.sha256}`);
    ids.add(entry.id);
    hashes.add(entry.sha256);
    if (
      !entry.name?.trim() ||
      !entry.nameZh?.trim() ||
      entry.nameZh.trim() === entry.name.trim() ||
      !entry.description?.trim() ||
      !entry.descriptionZh?.trim()
    ) {
      throw new Error(`incomplete bilingual metadata: ${entry.id}`);
    }
    if (!localizedMotionName(entry.name)) {
      throw new Error(`missing curated translation: ${entry.name}`);
    }
    const path = join(BUILTIN_ROOT, entry.fileName);
    if (!existsSync(path) || !statSync(path).isFile())
      throw new Error(`missing motion blob: ${entry.fileName}`);
    const bytes = readFileSync(path);
    const actual = createHash("sha256").update(bytes).digest("hex");
    if (actual !== entry.sha256) throw new Error(`motion hash mismatch: ${entry.id}`);
    const gltf = parseGlb(bytes, path);
    if (gltf.json.extensions?.VRMC_vrm_animation?.specVersion !== "1.0") {
      throw new Error(`motion is not VRMA 1.0: ${entry.id}`);
    }
    const animation = gltf.json.animations?.[0];
    if (!animation) throw new Error(`motion has no animation: ${entry.id}`);
    validateAnimationAccessors(gltf, animation, entry.id);
  }
  process.stdout.write(`Avatar Motion V4 catalog verified: ${catalog.entries.length} assets.\n`);
}
