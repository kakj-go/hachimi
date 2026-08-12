import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { format } from "prettier";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const catalogPath = join(repositoryRoot, "assets/avatar-motions-v5/catalog.json");
const reportPath = join(repositoryRoot, "docs/BUILTIN_VRMA_AUDIT.md");
const catalog = JSON.parse(readFileSync(catalogPath, "utf8"));

const REQUIRED_DEPENDENCIES = new Set();

const CORE = new Set([
  "builtin.clawatar.angry-gesture.94a2a266",
  "builtin.clawatar.annoyed-head-shake.5975c067",
  "builtin.clawatar.happy-hand-gesture.79fd8cf9",
  "builtin.clawatar.head-nod-yes.e6be0649",
  "builtin.clawatar.quick-informal-bow.a83dfc6d",
  "builtin.clawatar.relieved-sigh.5ccf9e32",
  "builtin.clawatar.shaking-head-no.2994e837",
  "builtin.clawatar.shrugging.d548d2e9",
  "builtin.clawatar.waving.8887b88a",
  "builtin.openmaiwaifu.appearing.b4240ae2",
  "builtin.openmaiwaifu.happy.ba2b5f00",
  "builtin.openmaiwaifu.laughing.893ae64f",
  "builtin.openmaiwaifu.waiting.3b2e83e2",
]);

const DM_REVIEW_DELETE = new Set(["dm_85", "dm_87"]);

const ACTIONS = new Map([
  ["Angry", "长时间跺脚、挥臂等生气表演"],
  ["Angry Gesture", "短促的生气抗议手势"],
  ["Annoyed Head Shake", "不耐烦地摇头"],
  ["Arms Hip Hop Dance", "以手臂动作为主的长段嘻哈舞"],
  ["Bboy Hip Hop Move", "短促的 B-boy 嘻哈招式"],
  ["Belly Dance", "长段肚皮舞表演"],
  ["Bellydancing", "另一段更长的肚皮舞表演"],
  ["Chicken Dance", "小鸡舞式的夸张娱乐动作"],
  ["Clapping", "双手鼓掌"],
  ["Crying", "低落并哭泣的全身反应"],
  ["Dancing Twerk", "以臀部动作为重点的舞蹈"],
  ["Defeat", "受挫、垂头丧气的反应"],
  ["Drunk Idle", "身体摇晃的醉酒待机"],
  ["Drunk Idle Variation", "另一种醉酒摇晃待机"],
  ["Happy Hand Gesture", "开心时的短手势"],
  ["Head Nod Yes", "点头表示同意"],
  ["Hip Hop Dancing", "短段嘻哈舞表演"],
  ["Hip Hop Dancing 2", "第二段嘻哈舞表演"],
  ["Hip Hop Dancing 3", "第三段嘻哈舞表演"],
  ["Hip Hop Dancing 4", "第四段嘻哈舞表演"],
  ["House Dancing", "长段浩室舞表演"],
  ["House Dancing 2", "第二段、更长的浩室舞表演"],
  ["Idle", "短周期的中性站立待机"],
  ["Jazz Dancing", "短段爵士舞表演"],
  ["Joyful Jump", "开心地跳起庆祝"],
  ["Leaning", "倚靠不存在的墙面或物体"],
  ["Loser", "表示失败或嘲讽的手势"],
  ["Macarena Dance", "玛卡莲娜舞表演"],
  ["Neck Stretching", "左右活动颈部"],
  ["No", "较完整地表达拒绝"],
  ["Quick Formal Bow", "快速正式鞠躬"],
  ["Quick Informal Bow", "快速随意鞠躬"],
  ["Rejected", "被拒绝后的失落反应"],
  ["Relieved Sigh", "放松并叹气"],
  ["reze dance hard", "角色主题的高强度长舞蹈"],
  ["Rumba Dancing", "短段伦巴舞表演"],
  ["Sad Idle", "低头沮丧的待机循环"],
  ["Shaking Head No", "快速摇头表示否定"],
  ["Shrugging", "耸肩表示不知道或无奈"],
  ["Silly Dancing", "短段搞怪舞蹈"],
  ["Singing", "唱歌时的全身伴随动作"],
  ["Snake Hip Hop Dance", "长段蛇形嘻哈舞"],
  ["Standing Greeting", "站立并做完整问候"],
  ["Swing Dancing", "长段摇摆舞表演"],
  ["Swing Dancing 2", "第二段摇摆舞表演"],
  ["Swing Dancing 3", "短段摇摆舞表演"],
  ["Talking", "说话时的身体伴随动作"],
  ["Talking On Phone", "持有不存在的手机并长时间通话"],
  ["Thankful", "合手或鞠身表达感谢"],
  ["Thinking", "托腮或停顿思考"],
  ["Waving", "挥手问候或告别"],
  ["Whatever Gesture", "摊手表达随意或无所谓"],
  ["appearing", "角色登场亮相"],
  ["cool liked", "酷系角色被喜欢后的回应"],
  ["cool waiting", "酷系自然待机循环"],
  ["energetic liked", "活力型开心回应"],
  ["energetic waiting", "活力型自然待机循环"],
  ["flamboyant liked", "华丽型开心回应"],
  ["flamboyant waiting", "华丽型自然待机循环"],
  ["gentleman liked", "绅士型开心回应"],
  ["gentleman waiting", "绅士型自然待机循环"],
  ["happy", "完整的开心全身反应"],
  ["ladylike liked", "淑女型开心回应"],
  ["ladylike waiting", "淑女型自然待机循环"],
  ["laughing", "带表情和身体动作的大笑"],
  ["liked", "较长的通用开心回应"],
  ["manly appearing", "帅气风格的登场亮相"],
  ["photobooth model pose", "面向拍照亭镜头摆模特姿势"],
  ["photobooth peace sign", "面向拍照亭镜头做胜利手势"],
  ["photobooth shoot", "配合拍照倒计时摆拍"],
  ["photobooth show full body", "后退或调整姿势展示全身"],
  ["photobooth spin", "面向拍照场景旋转展示"],
  ["photobooth squat", "面向拍照场景下蹲摆姿"],
  ["powerful liked", "力量型开心回应"],
  ["powerful waiting", "力量型自然待机循环"],
  ["shy liked", "害羞风格的开心回应"],
  ["shy waiting", "害羞风格的自然待机循环"],
  ["standard liked", "标准的短开心回应"],
  ["standard waiting", "标准自然待机循环"],
  ["stretching", "较完整的全身伸展"],
  ["waiting", "通用自然等待循环"],
  ["walk", "原地步态循环和根速度来源"],
  ["walk start", "从站立进入步态的起步片段"],
  ["walk stop", "从步态制动到站立的停步片段"],
  ["turn left", "带左向程序转角的落地转身"],
  ["turn right", "带右向程序转角的落地转身"],
  ["locomotion recover to idle", "移动被打断后回到中性待机"],
  ["action recover to idle", "动作被打断后回到中性待机"],
]);

function actionFor(entry) {
  return ACTIONS.get(entry.name) ?? entry.nameZh;
}

function sourceDmId(entry) {
  for (const path of entry.sourcePaths) {
    const match = /\/(dm_\d+)\.vrma$/i.exec(path);
    if (match) return match[1].toLowerCase();
  }
  return null;
}

function sourceCategoryFor(entry) {
  return ["idle-night", "idle-sit", "talking", "reaction", "oneshot", "dance", "idle"].find(
    (category) => entry.tags.includes(category),
  );
}

function recommendationFor(entry) {
  if (entry.derivedFromMotionId || REQUIRED_DEPENDENCIES.has(entry.id)) return "必须保留依赖";
  const dmId = sourceDmId(entry);
  if (dmId && DM_REVIEW_DELETE.has(dmId)) return "建议删除";
  if (CORE.has(entry.id)) return "保留核心";
  return "保留可选";
}

function fitFor(entry, recommendation) {
  if (recommendation === "建议删除") {
    if (entry.tags.includes("idle-sit")) return "D";
    return "C";
  }
  if (entry.derivedFromMotionId) return "B";
  if (entry.family === "locomotion" || entry.family === "recovery") return "A";
  if (entry.sourceProject.includes("OpenMaiWaifu")) {
    return entry.durationMs > 10000 && entry.family !== "idle" ? "B" : "A";
  }
  return recommendation === "保留核心" && entry.durationMs <= 6000 ? "B" : "C";
}

function reasonFor(entry, recommendation, fit) {
  if (entry.derivedFromMotionId)
    return `V5 ${entry.motionRole} 运行时片段，依赖 \`${entry.derivedFromMotionId}\``;
  if (REQUIRED_DEPENDENCIES.has(entry.id)) {
    return entry.family === "locomotion"
      ? "起步、停步和左右转共用该二进制与步态相位"
      : "两条 recovery 共用该二进制；同时是高覆盖待机";
  }
  const dmId = sourceDmId(entry);
  if (dmId && entry.tags.includes("idle-sit")) {
    return "上游明确为坐姿动作，当前透明桌面场景没有座椅和坐姿锚点";
  }
  if (dmId) {
    return `Clawatar 固定提交将 ${dmId} 标为 ${sourceCategoryFor(entry) ?? entry.family}；先作为可选动作进行视觉签收`;
  }
  if (recommendation === "保留核心" && entry.sourceProject === "Clawatar") {
    return "语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现";
  }
  if (recommendation === "保留核心") return "覆盖手指/表情，适合高频待机或直接交互";
  if (entry.sourceProject === "Clawatar") {
    return entry.family === "performance"
      ? "可作为低频娱乐动作，但不应进入默认高频调度"
      : "可播放但缺少表情、LookAt 和多数手指细节，适合作为扩展动作";
  }
  if (entry.family === "idle") return "高覆盖人格化待机，按模型气质启用，避免全部同时随机";
  if (entry.durationMs > 10000) return "技术覆盖完整但反馈过长，应低频使用并允许 safe-point 打断";
  if (fit === "A") return "骨骼、手指及多数面部通道覆盖完整，适合作为人格扩展";
  return "技术兼容，适合低频或特定人格使用";
}

function coverageFor(entry) {
  const face = `${entry.hasExpression ? "表情" : "无表情"}/${entry.hasLookAt ? "视线" : "无视线"}`;
  return `${entry.animatedBones.length} 骨 / ${entry.fingerBoneCount} 指骨 / ${face}`;
}

function formatSize(bytes) {
  return `${(bytes / 1024 / 1024).toFixed(2)} MiB`;
}

const rows = catalog.entries
  .map((entry) => {
    const recommendation = recommendationFor(entry);
    const fit = fitFor(entry, recommendation);
    return { entry, recommendation, fit, reason: reasonFor(entry, recommendation, fit) };
  })
  .sort(
    (left, right) =>
      left.entry.sourceProject.localeCompare(right.entry.sourceProject) ||
      left.entry.name.localeCompare(right.entry.name, "en", { numeric: true }),
  );

const recommendationCounts = Object.fromEntries(
  ["保留核心", "必须保留依赖", "保留可选", "建议删除"].map((name) => [
    name,
    rows.filter((row) => row.recommendation === name).length,
  ]),
);
const uniqueEntries = [...new Map(catalog.entries.map((entry) => [entry.sha256, entry])).values()];
const deleteHashes = new Set(
  rows.filter((row) => row.recommendation === "建议删除").map((row) => row.entry.sha256),
);
const deleteBytes = uniqueEntries
  .filter((entry) => deleteHashes.has(entry.sha256))
  .reduce((sum, entry) => sum + entry.sizeBytes, 0);
const totalBytes = uniqueEntries.reduce((sum, entry) => sum + entry.sizeBytes, 0);
const dmRows = rows
  .filter((row) => sourceDmId(row.entry))
  .sort(
    (left, right) =>
      Number(sourceDmId(left.entry).slice(3)) - Number(sourceDmId(right.entry).slice(3)),
  );
const dmAuditRows = [
  ...dmRows.map((row) => ({ ...row, sourceId: sourceDmId(row.entry) })),
  { sourceId: "dm_38", removed: true },
].sort((left, right) => Number(left.sourceId.slice(3)) - Number(right.sourceId.slice(3)));
const projects = [...new Set(rows.map((row) => row.entry.sourceProject))];
const dispositions = ["保留核心", "必须保留依赖", "保留可选", "建议删除"];

const lines = [
  "# 内置 VRMA 资源审计",
  "",
  `> 审计基线：\`assets/avatar-motions-v5/catalog.json\`（${rows.length} 个目录条目）。本报告由 \`scripts/generate-builtin-vrma-audit.mjs\` 生成。`,
  "",
  "> 默认模型基线：`VRoid 2639776812528692620`（SHA-256 `a9cced952d6671b51faffe578d32613a8ee927deee1851cff91fdbc4e6ae7d26`）。",
  "",
  "## 结论",
  "",
  `在此前 23 个产品排除项基础上，本轮继续移除 42 个上游动作和 5 个派生 locomotion 项。当前 ${rows.length} 项建议划分为 **${recommendationCounts["保留核心"]} 个核心动作 + ${recommendationCounts["必须保留依赖"]} 个运行时依赖动作**；${recommendationCounts["保留可选"]} 个动作默认不加载；另有 ${recommendationCounts["建议删除"]} 个坐姿 DM Motionpack 条目等待最终视觉确认后放弃。`,
  "",
  `Clawatar DM Motionpack 语义继续以固定提交的上游 catalog 为准；已排除的源文件不会进入内置包。当前保留 ${dmRows.length} 个 DM 条目，2 个坐姿放弃候选约 ${formatSize(deleteBytes)}，当前全部唯一 VRMA 约 ${formatSize(totalBytes)}。`,
  "",
  "同步脚本现在会读取固定提交中的 `public/animations/catalog.json`，因此对话、问候、亲昵、拒绝、待机等动作不会再被误命名或误分配到 gesture。",
  "",
  "### 来源与处置矩阵",
  "",
  `| 来源 | ${dispositions.join(" | ")} | 合计 |`,
  `|---|${dispositions.map(() => "---:").join("|")}|---:|`,
  ...projects.map((project) => {
    const projectRows = rows.filter((row) => row.entry.sourceProject === project);
    return `| ${project} | ${dispositions
      .map((disposition) => projectRows.filter((row) => row.recommendation === disposition).length)
      .join(" | ")} | ${projectRows.length} |`;
  }),
  `| **合计** | ${dispositions.map((disposition) => recommendationCounts[disposition]).join(" | ")} | **${rows.length}** |`,
  "",
  "### 产品级排除项",
  "",
  "| 分组 | 数量 | 已从同步源排除 |",
  "|---|---:|---|",
  "| 场景或道具不成立 | 8 | Leaning、Talking On Phone、6 个 photobooth 动作 |",
  "| 默认内容/人格不适合 | 4 | Dancing Twerk、2 个 Drunk Idle、reze dance hard |",
  "| 过长或重复动作 | 10 | Angry、Arms Hip Hop Dance、Bellydancing、Hip Hop Dancing 2/3/4、House Dancing 2、Snake Hip Hop Dance、Swing Dancing 1/2 |",
  "| 无法识别舞种 | 1 | `dm_38`：上游仅标为 Dance request，VRMA 内嵌动画名只有 AC_19 |",
  "| 本轮动作清理 | 42 | 用户确认移除的上游动作；同 SHA-256 的重复来源一并排除 |",
  "| 本轮派生清理 | 5 | walk start/stop、turn left/right、locomotion recovery |",
  "",
  "### DM Motionpack 源头结论",
  "",
  "- 来源固定为 [Clawatar `e7c40f1`](https://github.com/Dongping-Chen/Clawatar/tree/e7c40f1a7b4526c854d5219fbd18225f9504e10f)。语义来自同一提交的 `public/animations/catalog.json`，75/75 均匹配。",
  "- 文件由 `fbx2vrma-dm-converter` 生成。上游提交 `54771d4` 的说明是“replace Booth poses with 140 real DM Motionpack animations”，但没有登记 DM Motionpack 的外部下载地址或许可证名称。",
  "- `dm_38` 是唯一被上游归为 dance 的文件；上游仅写 `Dance request`，VRMA 内嵌动画名只有 `AC_19`，没有 extras 或原始舞种，因此已按无法识别项删除。",
  `- \`dm_85\`、\`dm_87\` 是两个需要座椅/坐姿锚点的坐姿伸展，列为下一轮放弃候选。其余 ${dmRows.length - 2} 项仍先放可选包，等待 Motion Lab 人工视觉签收。`,
  "",
  "| 源 ID | 恢复后的动作 | 上游类别 | 时长 | 覆盖 | 建议 |",
  "|---|---|---|---:|---|---|",
  ...dmAuditRows.map((row) =>
    row.removed
      ? "| `dm_38` | 舞蹈表演<br>Dance request | dance | 6.21s | 49 骨 / 30 指骨 / 无表情/无视线 | 已删除：无法识别舞种 |"
      : `| \`${row.sourceId}\` | ${row.entry.nameZh}<br>${row.entry.name} | ${sourceCategoryFor(row.entry) ?? row.entry.family} | ${(row.entry.durationMs / 1000).toFixed(2)}s | ${coverageFor(row.entry)} | ${row.recommendation} |`,
  ),
  "",
  "## 判定口径",
  "",
  "- **A**：在默认 VRM 上技术通过，骨骼覆盖和动作语义都适合当前桌宠场景。",
  "- **B**：技术兼容且语义可用，但缺少手指/表情/LookAt，或持续时间偏长，需要 Runtime V5 叠加反馈和安全打断。",
  "- **C**：可以播放，但重复、语义不明、过长或不适合高频桌宠调度。",
  "- **D**：依赖当前不存在的道具/场景，或内容定位不适合内置包。",
  "- **保留核心**：进入默认预加载或高频调度集合。",
  "- **必须保留依赖**：被 recovery 派生动作引用；替换依赖关系前不能删除。",
  "- **保留可选**：移入按需下载/默认不调度的动作包，不占核心资源和决策空间。",
  "- **建议删除**：从内置目录和同步源中移除；如确有用户需求，应由用户自定义导入承担。",
  "",
  "技术兼容结论来自现有集成测试：全部条目均可重定向并在默认 VRM 的起点、中点、末点采样，未出现 NaN 或无效四元数。它证明“能播”，不等于已逐帧人工确认视觉质量。产品适配结论依据动作语义、时长、通道覆盖、依赖场景、重复度和当前桌宠交互目标；最终删除前仍应在 Motion Lab 对拟保留核心集合做一次人工视觉签收。",
  "",
  "## 逐项清单",
  "",
  "| # | 动作 ID / 中文名 | 时长 | 家族 | 具体动作与用途 | 覆盖 | 适配 | 建议 | 依据 |",
  "|---:|---|---:|---|---|---|:---:|---|---|",
  ...rows.map(
    ({ entry, recommendation, fit, reason }, index) =>
      `| ${index + 1} | \`${entry.id}\`<br>${entry.nameZh} | ${(entry.durationMs / 1000).toFixed(2)}s | ${entry.family}/${entry.slot} | ${actionFor(entry)} | ${coverageFor(entry)} | ${fit} | ${recommendation} | ${reason} |`,
  ),
  "",
  "## 对动作连贯性的直接影响",
  "",
  "清理目录不能替代 Runtime V5 的 TransitionPlanner 和惯性化，但已经消除了错误切换来源：BehaviorScheduler 不再把 DM 的对话、反应和待机当普通舞蹈，也不会选择已移除的长动作、空气倚靠或无手机通话。核心集合应按以下方式使用：",
  "",
  "| 产品意图 | 首选动作 | 调度说明 |",
  "|---|---|---|",
  "| 首次显示 | `appearing` 覆盖在 `waiting` 上 | 两个动作都准备完成且首帧已采样后才显示模型 |",
  "| 默认 idle | `waiting` | 唯一常驻 base；一次性动作淡出后自然露出该层 |",
  "| 通用沟通 | `waving`、`head nod yes`、`shaking head no`、`shrugging` | 都是短且语义明确的离散动作；同类只保留最新请求 |",
  "| 语音 | LipSync、表情和注视层 | 已删除通用 `talking` VRMA，避免全身动作抢占 base |",
  "| 移动 | 无内置动作 | locomotion slot 保留为扩展点，不以待机滑动代替行走 |",
  "| 打断恢复 | `action_recover_to_idle` | 取 `waiting` 前 260ms，之后继续由常驻 base 接管 |",
  "",
  "## 缺失的核心 VRMA",
  "",
  "当前目录即使按本报告清理后，仍不能单靠现有 VRMA 完成计划中的全部“上手感”。下一批资源应优先补齐：",
  "",
  "1. `touch_react_in`、`pet_loop`、`pet_release`：现有 liked 动作都是一次性开心回应，无法表达持续抚摸的输入开始、保持和自然释放。",
  "2. `speech_upper_body_neutral`：新资源应只占上身并可按语音能量连续缩放，避免抢占 waiting 和触摸。",
  "3. 成套 locomotion：只有 walk start/loop/stop、turn 和 recover 都具备独立的预备、制动、重心转移后才重新引入。",
  "4. 真正制作的 `action_recover_to_idle`：当前片段只是 `waiting` 的前 260ms，职责上可用但不是针对任意源姿势设计的恢复动作。",
  "5. `idle_micro_shift` 与 `interaction_recover`：用于长时间桌面停留中的小幅重心变化，以及触摸/语音结束后的短收势，减少每次都回到同一明显关键帧。",
  "",
  "在这些专用资源完成前，动作恢复片段必须保留，但适配等级按 B 处理：技术正确、能防止 T Pose，不代表重心、脚接触和收势已经达到最终视觉标准。",
  "",
  "## 后续清理顺序",
  "",
  "1. 在 Motion Lab 人工确认 `dm_85`、`dm_87`；确认后加入同步排除并删除无引用 blob。",
  `2. 对其余 ${dmRows.length - 2} 个 DM 动作按 reaction、idle、speech、gesture 分批预览，只将明显优于现有核心动作的少量条目升级为核心。`,
  "3. 将保留可选项拆为独立动作包，默认不预加载、不参与自主随机调度，仅由明确命令触发。",
  "4. 对最终核心集合执行 `idle → action → interaction → recovery → idle` 矩阵，并人工检查裙摆、长发、手指穿插、脚底漂移和镜头出界。",
  "",
  "## 对当前模型的总体判断",
  "",
  "OpenMaiWaifu 系列通常覆盖 49–52 根 humanoid 骨骼、28–30 根手指骨，并经常携带表情和 LookAt，和当前默认二次元 VRM 的表现能力更匹配。Clawatar 的原命名动作大多只有约 20–22 根身体骨骼；DM Motionpack 覆盖范围为 17–51 根骨骼，其中一部分带手指轨道，但两组都没有表情和 LookAt，仍需 Runtime V5 的表情、注视与交互反馈层补足。",
  "",
  "不同用户 VRM 的体型、裙装、长发和骨骼比例仍可能改变结果，因此“A”不是对所有模型的永久保证。建议将模型签名维度的 Motion Lab 指标保留为资源准入条件，而不是把默认模型的一次通过扩展成全模型结论。",
  "",
];

writeFileSync(reportPath, await format(`${lines.join("\n")}\n`, { parser: "markdown" }));
process.stdout.write(
  `Generated ${reportPath}: ${rows.length} rows, ${recommendationCounts["建议删除"]} delete candidates.\n`,
);
