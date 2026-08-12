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
import { format } from "prettier";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(SCRIPT_DIR, "..");
const OUTPUT_ROOT = join(ROOT, "assets", "avatar-motions-v5");
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
    motionCatalogs: ["public/animations/catalog.json"],
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
  "clawatar/public/animations/0_Angry.vrma",
  "clawatar/public/animations/1_Arms Hip Hop Dance.vrma",
  "clawatar/public/animations/5_Bellydancing.vrma",
  "clawatar/public/animations/25_Dancing Twerk.vrma",
  "clawatar/public/animations/29_Drunk Idle Variation.vrma",
  "clawatar/public/animations/30_Drunk Idle.vrma",
  "clawatar/public/animations/42_Hip Hop Dancing_2.vrma",
  "clawatar/public/animations/43_Hip Hop Dancing_3.vrma",
  "clawatar/public/animations/44_Hip Hop Dancing_4.vrma",
  "clawatar/public/animations/46_House Dancing_2.vrma",
  "clawatar/public/animations/78_Snake Hip Hop Dance.vrma",
  "clawatar/public/animations/83_Swing Dancing.vrma",
  "clawatar/public/animations/84_Swing Dancing_2.vrma",
  "clawatar/public/animations/127_Leaning.vrma",
  "clawatar/public/animations/155_Talking On Phone.vrma",
  "clawatar/public/animations/dm_38.vrma",
  "clawatar/public/animations/reze_dance_hard.vrma",
  "openmaiwaifu/public/motions/photobooth-model-pose.vrma",
  "openmaiwaifu/public/motions/photobooth-peace-sign.vrma",
  "openmaiwaifu/public/motions/photobooth-shoot.vrma",
  "openmaiwaifu/public/motions/photobooth-show-full-body.vrma",
  "openmaiwaifu/public/motions/photobooth-spin.vrma",
  "openmaiwaifu/public/motions/photobooth-squat.vrma",
  "openmaiwaifu/assets/motions/test-pixiv.vrma",
  "openmaiwaifu/public/motions/photobooth-greeting.vrma",
  "clawatar/public/animations/dm_41.vrma",
  "clawatar/public/animations/19_Clapping.vrma",
  "clawatar/public/animations/22_Crying.vrma",
  "clawatar/public/animations/23_Crying_2.vrma",
  "clawatar/public/animations/26_Defeat.vrma",
  "clawatar/public/animations/dm_0.vrma",
  "clawatar/public/animations/dm_5.vrma",
  "clawatar/public/animations/dm_6.vrma",
  "clawatar/public/animations/dm_7.vrma",
  "clawatar/public/animations/dm_13.vrma",
  "clawatar/public/animations/dm_14.vrma",
  "clawatar/public/animations/dm_15.vrma",
  "clawatar/public/animations/dm_46.vrma",
  "clawatar/public/animations/dm_127.vrma",
  "clawatar/public/animations/41_Hip Hop Dancing.vrma",
  "clawatar/public/animations/45_House Dancing.vrma",
  "clawatar/public/animations/119_Idle.vrma",
  "clawatar/public/animations/56_No.vrma",
  "clawatar/public/animations/137_Quick Formal Bow.vrma",
  "clawatar/public/animations/142_Sad Idle.vrma",
  "clawatar/public/animations/71_Singing.vrma",
  "clawatar/public/animations/dm_114.vrma",
  "clawatar/public/animations/dm_129.vrma",
  "clawatar/public/animations/dm_110.vrma",
  "clawatar/public/animations/79_Standing Greeting.vrma",
  "clawatar/public/animations/dm_138.vrma",
  "clawatar/public/animations/86_Talking.vrma",
  "clawatar/public/animations/156_Thankful.vrma",
  "clawatar/public/animations/dm_3.vrma",
  "clawatar/public/animations/88_Thinking.vrma",
  "clawatar/public/animations/dm_111.vrma",
  "clawatar/public/animations/93_Whatever Gesture.vrma",
  "clawatar/public/animations/dm_59.vrma",
  "openmaiwaifu/public/motions/energetic-liked.vrma",
  "openmaiwaifu/public/motions/energetic-waiting.vrma",
  "openmaiwaifu/public/motions/flamboyant-waiting.vrma",
  "openmaiwaifu/assets/motions/powerful-liked.vrma",
  "openmaiwaifu/public/motions/powerful-liked.vrma",
  "openmaiwaifu/assets/motions/powerful-waiting.vrma",
  "openmaiwaifu/public/motions/powerful-waiting.vrma",
  "openmaiwaifu/assets/motions/shy-liked.vrma",
  "openmaiwaifu/public/motions/shy-liked.vrma",
  "openmaiwaifu/assets/motions/shy-waiting.vrma",
  "openmaiwaifu/public/motions/shy-waiting.vrma",
  "openmaiwaifu/assets/motions/standard-liked.vrma",
  "openmaiwaifu/public/motions/standard-liked.vrma",
  "openmaiwaifu/assets/motions/standard-waiting.vrma",
  "openmaiwaifu/public/motions/standard-waiting.vrma",
  "openmaiwaifu/assets/motions/walk.vrma",
]);

const SOURCE_TRIGGER_ZH = new Map(
  Object.entries({
    "Angry shhh": "生气地示意安静",
    "Arms crossed X — say no": "双臂交叉表示拒绝",
    "Blow kiss": "飞吻",
    "Cat pose": "猫咪姿势",
    "Checking time": "查看时间",
    "Cheer/encourage": "加油鼓励",
    "Cheer/fighting!": "挥拳加油",
    "Chill pose": "放松姿势",
    "Comforting tired/sleepy user": "安慰疲惫的用户",
    "Cool pose": "酷系姿势",
    "Cool standby pose": "酷系待机",
    "Cute jumping": "可爱跳跃",
    "Cute peace sign": "可爱胜利手势",
    "Cute pose": "可爱姿势",
    "Cute standby": "可爱待机",
    "Dance request": "舞蹈表演",
    "Dog pose": "小狗姿势",
    "During conversation": "对话伴随动作",
    "Encouraging user": "鼓励用户",
    "Energetic cheering": "活力加油",
    "Energetic pose": "活力姿势",
    "Energetic standby": "活力待机",
    "Exercising standby": "运动式待机",
    "Fidgeting fingers — made a mistake": "犯错后不安地摆弄手指",
    "Finger to lips shhh": "手指抵唇示意安静",
    "Greeting user": "问候用户",
    "Hands on hips idle": "叉腰待机",
    "Happy small jump": "开心小跳",
    "Happy standing idle": "开心站立待机",
    "Heart hands": "双手比心",
    "Little tiger/cat pose": "小老虎或猫咪姿势",
    "Looking at watch": "看手表",
    "Nice standing idle": "自然站立待机",
    "Peace sign": "胜利手势",
    "Peace sign variant": "胜利手势变化",
    "Pointing/directing — thinking": "指向并思考",
    "Presenting/showing something": "介绍或展示内容",
    "Salute greeting": "敬礼问候",
    "Shy, hands on chest": "双手放胸前的害羞反应",
    "Side blow kiss": "侧身飞吻",
    "Singing pose": "唱歌姿势",
    "Sitting stretch": "坐姿伸展",
    "Sitting yawn, sleepy": "坐姿打哈欠",
    Sleepy: "困倦待机",
    "Sleepy standing": "困倦站立待机",
    "Slightly depressed/tired/sleepy": "轻微低落或疲惫待机",
    "Slower salute greeting": "缓慢敬礼问候",
    "Standing idle": "站立待机",
    "Standing idle variant": "站立待机变化",
    "Standing impatient": "不耐烦站立待机",
    "Standing relaxed": "放松站立待机",
    "Standing stretch": "站立伸展",
    "Stretching idle": "伸展待机",
    "Thinking or acknowledging": "思考或确认",
    "Tired/weary": "疲惫待机",
    "User assigns task — received!": "确认收到用户任务",
    "User bullies, whiny protest": "被欺负后的委屈抗议",
    "User pokes or teases": "被戳或逗弄后的反应",
    "Very cute greeting": "可爱问候",
    "Warm-up idle": "热身待机",
    "Yawning/stretching idle": "打哈欠并伸展的待机",
  }),
);

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
    "walk start": "起步",
    "walk stop": "停步",
    "turn left": "左转",
    "turn right": "右转",
    "locomotion recover to idle": "移动恢复待机",
    "action recover to idle": "动作恢复待机",
    Waving: "挥手",
    "Whatever Gesture": "随意手势",
  }),
);

const checkOnly = process.argv.includes("--check");
if (checkOnly) {
  checkCatalog();
} else {
  await syncCatalog();
}

async function syncCatalog() {
  const temporaryRoot = mkdtempSync(join(tmpdir(), "hachimi-motion-v5-"));
  const records = [];
  try {
    for (const source of SOURCES) {
      const checkout = join(temporaryRoot, source.id);
      clonePinnedSource(source, checkout);
      const motionMetadata = loadSourceMotionMetadata(source, checkout);
      for (const sourcePath of source.paths) {
        const absolute = join(checkout, sourcePath);
        if (!existsSync(absolute)) continue;
        for (const path of walkFiles(absolute).filter(
          (value) => extname(value).toLowerCase() === ".vrma",
        )) {
          const normalizedSourcePath = `${source.id}/${relative(checkout, path).replaceAll("\\", "/")}`;
          if (EXCLUDED_SOURCE_PATHS.has(normalizedSourcePath)) continue;
          records.push(
            readMotionRecord(source, checkout, path, motionMetadata.get(normalizedSourcePath)),
          );
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
    const waiting = entries.find((entry) => entry.name.trim().toLowerCase() === "waiting");
    if (!waiting) throw new Error("pinned sources do not contain the required waiting motion");
    const fallbackMotionId = waiting.id;
    for (const entry of entries) entry.fallbackMotionId = fallbackMotionId;
    entries.push(derivedActionRecoveryEntry(waiting));
    entries.sort((left, right) => left.id.localeCompare(right.id));
    const catalog = {
      schemaVersion: 2,
      specVersion: "1.0",
      generatedAt: new Date().toISOString(),
      sources: SOURCES.map(({ id, label, repository, commit }) => ({
        id,
        label,
        repository: repository.replace(/\.git$/, ""),
        commit,
      })),
      transitionProfiles: transitionProfiles(),
      entries,
    };
    writeFileSync(CATALOG_PATH, await format(JSON.stringify(catalog), { parser: "json" }));
    writeNotices();
    checkCatalog();
    process.stdout.write(
      `Avatar Motion V5: ${records.length} source files, ${entries.length} unique VRMA assets.\n`,
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

function loadSourceMotionMetadata(source, checkout) {
  const result = new Map();
  for (const relativeCatalogPath of source.motionCatalogs ?? []) {
    const catalogPath = join(checkout, relativeCatalogPath);
    const catalog = JSON.parse(readFileSync(catalogPath, "utf8"));
    if (!Array.isArray(catalog.animations)) {
      throw new Error(`${catalogPath}: expected an animations array`);
    }
    const catalogDirectory = dirname(relativeCatalogPath).replaceAll("\\", "/");
    for (const metadata of catalog.animations) {
      if (!metadata?.file || !metadata?.trigger || !metadata?.category) continue;
      if (!/^dm_\d+\.vrma$/i.test(metadata.file)) continue;
      const sourcePath = `${source.id}/${catalogDirectory}/${metadata.file}`;
      if (result.has(sourcePath))
        throw new Error(`${catalogPath}: duplicate metadata for ${sourcePath}`);
      result.set(sourcePath, metadata);
    }
  }
  return result;
}

function readMotionRecord(source, checkout, path, sourceMetadata) {
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
  if (/^dm_\d+$/i.test(stem) && !sourceMetadata) {
    throw new Error(`${path}: missing source motion metadata`);
  }
  const category = sourceMetadata
    ? normalizeSourceCategory(sourceMetadata.category)
    : inferCategory(stem);
  const title = sourceMetadata?.trigger ?? cleanTitle(stem);
  const nameZh = sourceMetadata ? SOURCE_TRIGGER_ZH.get(sourceMetadata.trigger) : undefined;
  if (sourceMetadata && !nameZh) {
    throw new Error(
      `${path}: missing Chinese translation for source trigger ${sourceMetadata.trigger}`,
    );
  }
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
    nameZh,
    category,
    tags: sourceMetadata
      ? [
          ...new Set(
            [
              category,
              sourceMetadata.category,
              sourceMetadata.subcategory,
              ...(sourceMetadata.tags ?? []),
            ].filter(Boolean),
          ),
        ]
      : inferTags(stem, category),
    playbackMode: sourceMetadata
      ? sourceMetadata.loop
        ? "loop"
        : "once"
      : inferPlaybackMode(stem, category),
    rootMode: inferRootMode(sourceMetadata?.trigger ?? stem, category),
    channels: inferChannels(animatedBones),
    animatedBones,
    fingerBoneCount: animatedBones.filter((bone) => FINGER_PATTERN.test(bone)).length,
    hasExpression: Boolean(extension.expressions),
    hasLookAt: Boolean(extension.lookAt),
    mirrorable: inferMirrorable(stem),
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
  const nameZh = record.nameZh ?? localizedMotionName(record.title);
  const description = motionDescription(record.title, record.category);
  const descriptionZh = motionDescriptionZh(nameZh, record.category);
  return {
    id,
    source: "builtin",
    analysisStatus: "ready",
    protected: true,
    name: record.title,
    nameZh,
    description,
    descriptionZh,
    fileName,
    sha256: record.sha256,
    sizeBytes: record.sizeBytes,
    durationMs: record.durationMs,
    family: record.category,
    tags: record.tags,
    loopMode: record.playbackMode,
    rootMode: record.rootMode,
    slot: slotForFamily(record.category),
    channelMask: record.channels,
    transitionProfileId: profileForFamily(record.category),
    fallbackMotionId: "",
    animatedBones: record.animatedBones,
    fingerBoneCount: record.fingerBoneCount,
    hasFingerMotion: record.fingerBoneCount > 0,
    hasExpression: record.hasExpression,
    hasLookAt: record.hasLookAt,
    mirrorable: record.mirrorable,
    sourceProject: record.sourceProjects.join(" / "),
    sourcePaths: record.sourcePaths,
    warnings: record.warnings,
  };
}

function localizedMotionName(title) {
  const translated = MOTION_NAME_ZH.get(title) ?? SOURCE_TRIGGER_ZH.get(title);
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

function normalizeSourceCategory(category) {
  if (category === "idle" || category === "idle-night" || category === "idle-sit") return "idle";
  if (category === "talking") return "speech";
  if (category === "reaction") return "reaction";
  if (category === "dance") return "performance";
  if (category === "oneshot") return "gesture";
  throw new Error(`unsupported source motion category: ${category}`);
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

function slotForFamily(family) {
  if (family === "idle") return "base";
  if (family === "locomotion") return "locomotion";
  if (family === "speech") return "speech";
  return "action";
}

function profileForFamily(family) {
  if (family === "idle") return "idle.standard";
  if (family === "locomotion") return "locomotion.contact";
  if (family === "speech") return "speech.upper-body";
  if (family === "reaction") return "reaction.responsive";
  return "action.standard";
}

function transitionProfiles() {
  const profile = (
    id,
    family,
    preferredDurationMs,
    minimumDurationMs,
    maximumDurationMs,
    interruptPolicy,
    syncGroup,
  ) => ({
    id,
    family,
    preferredDurationMs,
    minimumDurationMs,
    maximumDurationMs,
    interruptPolicy,
    blendProfileId: "dead_blend.v1",
    syncGroup: syncGroup ?? null,
    entryWindows: [{ startMs: 0, endMs: 120 }],
    exitWindows: [],
    channelMask: ["full_body"],
    inertialHalfLives: {
      rootMs: 100,
      bodyMs: 80,
      armsMs: 65,
      lookAtMs: 60,
      expressionMs: 50,
    },
  });
  return [
    profile("idle.standard", "idle", 180, 100, 240, "safe_point"),
    profile("reaction.responsive", "reaction", 90, 55, 120, "safe_point"),
    profile("action.standard", "gesture", 150, 80, 240, "safe_point"),
    profile("speech.upper-body", "speech", 100, 60, 120, "safe_point"),
    profile("locomotion.contact", "locomotion", 100, 70, 120, "safe_point", "locomotion.feet"),
    profile("recovery.fast", "recovery", 90, 55, 120, "immediate"),
    profile("unknown.conservative", "unknown", 200, 120, 280, "safe_point"),
  ];
}

function derivedActionRecoveryEntry(waiting) {
  return {
    ...waiting,
    id: "builtin.hachimi.action-recover-to-idle.v1",
    name: "action recover to idle",
    nameZh: "动作恢复待机",
    description: "A short neutral recovery segment.",
    descriptionZh: "回到等待姿势的短恢复片段。",
    protected: true,
    family: "recovery",
    tags: ["recovery", "action_recover_to_idle"],
    loopMode: "once",
    derivedFromMotionId: waiting.id,
    motionRole: "action_recover_to_idle",
    sourceStartMs: 0,
    sourceEndMs: 260,
    proceduralYawDegrees: null,
    durationMs: 260,
    slot: "action",
    transitionProfileId: "recovery.fast",
    sourceProject: `Hachimi derived from ${waiting.sourceProject}`,
    sourcePaths: [...waiting.sourcePaths, "derived:action_recover_to_idle"],
    warnings: [...(waiting.warnings ?? []), "derived_motion_segment"],
  };
}

function cleanTitle(stem) {
  return stem.replace(/^\d+_/, "").replaceAll(/[-_]+/g, " ").replaceAll(/\s+/g, " ").trim();
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
    "# Avatar Motion V5 sources",
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
    catalog.schemaVersion !== 2 ||
    catalog.specVersion !== "1.0" ||
    !Array.isArray(catalog.entries) ||
    !Array.isArray(catalog.transitionProfiles)
  ) {
    throw new Error("invalid Avatar Motion V5 catalog header");
  }
  const ids = new Set();
  const hashEntries = new Map();
  for (const entry of catalog.entries) {
    if (ids.has(entry.id)) throw new Error(`duplicate motion id: ${entry.id}`);
    ids.add(entry.id);
    const sharedEntries = hashEntries.get(entry.sha256) ?? [];
    sharedEntries.push(entry);
    hashEntries.set(entry.sha256, sharedEntries);
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
    const hasDmSource = entry.sourcePaths?.some((sourcePath) =>
      /\/dm_\d+\.vrma$/i.test(sourcePath),
    );
    if (
      hasDmSource &&
      (!SOURCE_TRIGGER_ZH.has(entry.name) || /^Dance Motion \d+$/i.test(entry.name))
    ) {
      throw new Error(`DM Motionpack semantics were not preserved: ${entry.id}`);
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
  for (const [hash, sharedEntries] of hashEntries) {
    if (sharedEntries.length < 2) continue;
    const canonicalEntries = sharedEntries.filter((entry) => !entry.derivedFromMotionId);
    if (canonicalEntries.length !== 1) {
      throw new Error(`motion blob ${hash} must have exactly one canonical entry`);
    }
    const canonical = canonicalEntries[0];
    for (const entry of sharedEntries) {
      if (entry.id === canonical.id) continue;
      if (
        entry.derivedFromMotionId !== canonical.id ||
        entry.fileName !== canonical.fileName ||
        !entry.warnings?.includes("derived_motion_segment")
      ) {
        throw new Error(
          `${entry.id} shares motion blob ${hash} without deriving from ${canonical.id}`,
        );
      }
    }
  }
  process.stdout.write(`Avatar Motion V5 catalog verified: ${catalog.entries.length} assets.\n`);
}
