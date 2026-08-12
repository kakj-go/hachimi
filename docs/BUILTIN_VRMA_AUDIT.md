# 内置 VRMA 资源审计

> 审计基线：`assets/avatar-motions-v5/catalog.json`（93 个目录条目）。本报告由 `scripts/generate-builtin-vrma-audit.mjs` 生成。

> 默认模型基线：`VRoid 2639776812528692620`（SHA-256 `a9cced952d6671b51faffe578d32613a8ee927deee1851cff91fdbc4e6ae7d26`）。

## 结论

在此前 23 个产品排除项基础上，本轮继续移除 42 个上游动作和 5 个派生 locomotion 项。当前 93 项建议划分为 **13 个核心动作 + 1 个运行时依赖动作**；77 个动作默认不加载；另有 2 个坐姿 DM Motionpack 条目等待最终视觉确认后放弃。

Clawatar DM Motionpack 语义继续以固定提交的上游 catalog 为准；已排除的源文件不会进入内置包。当前保留 57 个 DM 条目，2 个坐姿放弃候选约 0.34 MiB，当前全部唯一 VRMA 约 14.28 MiB。

同步脚本现在会读取固定提交中的 `public/animations/catalog.json`，因此对话、问候、亲昵、拒绝、待机等动作不会再被误命名或误分配到 gesture。

### 来源与处置矩阵

| 来源                              | 保留核心 | 必须保留依赖 | 保留可选 | 建议删除 |   合计 |
| --------------------------------- | -------: | -----------: | -------: | -------: | -----: |
| Clawatar                          |        9 |            0 |       67 |        2 |     78 |
| Hachimi derived from OpenMaiWaifu |        0 |            1 |        0 |        0 |      1 |
| OpenMaiWaifu                      |        4 |            0 |       10 |        0 |     14 |
| **合计**                          |       13 |            1 |       77 |        2 | **93** |

### 产品级排除项

| 分组                | 数量 | 已从同步源排除                                                                                                          |
| ------------------- | ---: | ----------------------------------------------------------------------------------------------------------------------- |
| 场景或道具不成立    |    8 | Leaning、Talking On Phone、6 个 photobooth 动作                                                                         |
| 默认内容/人格不适合 |    4 | Dancing Twerk、2 个 Drunk Idle、reze dance hard                                                                         |
| 过长或重复动作      |   10 | Angry、Arms Hip Hop Dance、Bellydancing、Hip Hop Dancing 2/3/4、House Dancing 2、Snake Hip Hop Dance、Swing Dancing 1/2 |
| 无法识别舞种        |    1 | `dm_38`：上游仅标为 Dance request，VRMA 内嵌动画名只有 AC_19                                                            |
| 本轮动作清理        |   42 | 用户确认移除的上游动作；同 SHA-256 的重复来源一并排除                                                                   |
| 本轮派生清理        |    5 | walk start/stop、turn left/right、locomotion recovery                                                                   |

### DM Motionpack 源头结论

- 来源固定为 [Clawatar `e7c40f1`](https://github.com/Dongping-Chen/Clawatar/tree/e7c40f1a7b4526c854d5219fbd18225f9504e10f)。语义来自同一提交的 `public/animations/catalog.json`，75/75 均匹配。
- 文件由 `fbx2vrma-dm-converter` 生成。上游提交 `54771d4` 的说明是“replace Booth poses with 140 real DM Motionpack animations”，但没有登记 DM Motionpack 的外部下载地址或许可证名称。
- `dm_38` 是唯一被上游归为 dance 的文件；上游仅写 `Dance request`，VRMA 内嵌动画名只有 `AC_19`，没有 extras 或原始舞种，因此已按无法识别项删除。
- `dm_85`、`dm_87` 是两个需要座椅/坐姿锚点的坐姿伸展，列为下一轮放弃候选。其余 55 项仍先放可选包，等待 Motion Lab 人工视觉签收。

| 源 ID    | 恢复后的动作                                               | 上游类别   |   时长 | 覆盖                            | 建议                 |
| -------- | ---------------------------------------------------------- | ---------- | -----: | ------------------------------- | -------------------- |
| `dm_2`   | 鼓励用户<br>Encouraging user                               | reaction   |  5.63s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_4`   | 问候用户<br>Greeting user                                  | reaction   |  4.50s | 19 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_8`   | 被戳或逗弄后的反应<br>User pokes or teases                 | reaction   |  3.25s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_9`   | 被欺负后的委屈抗议<br>User bullies, whiny protest          | reaction   |  3.58s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_10`  | 敬礼问候<br>Salute greeting                                | reaction   |  3.46s | 17 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_11`  | 缓慢敬礼问候<br>Slower salute greeting                     | reaction   |  3.67s | 23 骨 / 4 指骨 / 无表情/无视线  | 保留可选             |
| `dm_12`  | 确认收到用户任务<br>User assigns task — received!          | reaction   |  3.17s | 20 骨 / 1 指骨 / 无表情/无视线  | 保留可选             |
| `dm_17`  | 轻微低落或疲惫待机<br>Slightly depressed/tired/sleepy      | idle-night |  7.38s | 19 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_19`  | 开心小跳<br>Happy small jump                               | oneshot    |  7.38s | 19 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_20`  | 安慰疲惫的用户<br>Comforting tired/sleepy user             | reaction   |  4.50s | 19 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_21`  | 侧身飞吻<br>Side blow kiss                                 | reaction   |  6.17s | 19 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_22`  | 打哈欠并伸展的待机<br>Yawning/stretching idle              | idle       |  8.08s | 34 骨 / 15 指骨 / 无表情/无视线 | 保留可选             |
| `dm_23`  | 酷系待机<br>Cool standby pose                              | idle       |  6.54s | 44 骨 / 25 指骨 / 无表情/无视线 | 保留可选             |
| `dm_24`  | 可爱待机<br>Cute standby                                   | idle       |  6.54s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_26`  | 可爱胜利手势<br>Cute peace sign                            | reaction   |  3.21s | 41 骨 / 22 指骨 / 无表情/无视线 | 保留可选             |
| `dm_27`  | 双臂交叉表示拒绝<br>Arms crossed X — say no                | reaction   |  4.88s | 43 骨 / 24 指骨 / 无表情/无视线 | 保留可选             |
| `dm_28`  | 加油鼓励<br>Cheer/encourage                                | reaction   |  3.58s | 46 骨 / 27 指骨 / 无表情/无视线 | 保留可选             |
| `dm_29`  | 双手比心<br>Heart hands                                    | reaction   |  3.33s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_30`  | 胜利手势<br>Peace sign                                     | reaction   |  9.79s | 19 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_31`  | 胜利手势变化<br>Peace sign variant                         | reaction   |  7.58s | 18 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_32`  | 可爱跳跃<br>Cute jumping                                   | oneshot    |  6.21s | 19 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_33`  | 酷系姿势<br>Cool pose                                      | idle       |  6.04s | 43 骨 / 24 指骨 / 无表情/无视线 | 保留可选             |
| `dm_38`  | 舞蹈表演<br>Dance request                                  | dance      |  6.21s | 49 骨 / 30 指骨 / 无表情/无视线 | 已删除：无法识别舞种 |
| `dm_39`  | 可爱问候<br>Very cute greeting                             | reaction   |  7.46s | 46 骨 / 27 指骨 / 无表情/无视线 | 保留可选             |
| `dm_40`  | 犯错后不安地摆弄手指<br>Fidgeting fingers — made a mistake | reaction   |  6.50s | 39 骨 / 20 指骨 / 无表情/无视线 | 保留可选             |
| `dm_42`  | 手指抵唇示意安静<br>Finger to lips shhh                    | reaction   |  6.50s | 46 骨 / 27 指骨 / 无表情/无视线 | 保留可选             |
| `dm_43`  | 猫咪姿势<br>Cat pose                                       | reaction   |  5.92s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_44`  | 胜利手势<br>Peace sign                                     | reaction   |  4.71s | 41 骨 / 22 指骨 / 无表情/无视线 | 保留可选             |
| `dm_45`  | 挥拳加油<br>Cheer/fighting!                                | reaction   |  3.21s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_47`  | 小狗姿势<br>Dog pose                                       | reaction   |  4.42s | 19 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_48`  | 猫咪姿势<br>Cat pose                                       | reaction   |  5.38s | 45 骨 / 26 指骨 / 无表情/无视线 | 保留可选             |
| `dm_51`  | 双手放胸前的害羞反应<br>Shy, hands on chest                | reaction   |  6.54s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_52`  | 生气地示意安静<br>Angry shhh                               | reaction   |  5.75s | 48 骨 / 29 指骨 / 无表情/无视线 | 保留可选             |
| `dm_53`  | 活力加油<br>Energetic cheering                             | reaction   |  4.79s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_56`  | 可爱姿势<br>Cute pose                                      | reaction   |  4.96s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_57`  | 小老虎或猫咪姿势<br>Little tiger/cat pose                  | reaction   |  8.00s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_63`  | 运动式待机<br>Exercising standby                           | idle       |  6.50s | 45 骨 / 26 指骨 / 无表情/无视线 | 保留可选             |
| `dm_82`  | 伸展待机<br>Stretching idle                                | idle       | 13.63s | 21 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_85`  | 坐姿伸展<br>Sitting stretch                                | idle-sit   | 15.29s | 21 骨 / 0 指骨 / 无表情/无视线  | 建议删除             |
| `dm_86`  | 站立伸展<br>Standing stretch                               | idle       | 12.46s | 21 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_87`  | 坐姿伸展<br>Sitting stretch                                | idle-sit   | 20.29s | 21 骨 / 0 指骨 / 无表情/无视线  | 建议删除             |
| `dm_88`  | 热身待机<br>Warm-up idle                                   | idle       | 14.33s | 21 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_89`  | 热身待机<br>Warm-up idle                                   | idle       | 15.00s | 21 骨 / 0 指骨 / 无表情/无视线  | 保留可选             |
| `dm_90`  | 介绍或展示内容<br>Presenting/showing something             | talking    | 12.88s | 43 骨 / 24 指骨 / 无表情/无视线 | 保留可选             |
| `dm_97`  | 唱歌姿势<br>Singing pose                                   | reaction   | 22.67s | 34 骨 / 15 指骨 / 无表情/无视线 | 保留可选             |
| `dm_101` | 放松姿势<br>Chill pose                                     | idle       | 12.58s | 37 骨 / 18 指骨 / 无表情/无视线 | 保留可选             |
| `dm_108` | 指向并思考<br>Pointing/directing — thinking                | reaction   |  8.17s | 49 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_120` | 站立待机<br>Standing idle                                  | idle       | 15.25s | 45 骨 / 24 指骨 / 无表情/无视线 | 保留可选             |
| `dm_121` | 放松站立待机<br>Standing relaxed                           | idle       | 15.96s | 51 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_122` | 站立待机<br>Standing idle                                  | idle       | 16.46s | 47 骨 / 26 指骨 / 无表情/无视线 | 保留可选             |
| `dm_123` | 自然站立待机<br>Nice standing idle                         | idle       | 13.50s | 51 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_124` | 站立待机变化<br>Standing idle variant                      | idle       | 16.29s | 45 骨 / 24 指骨 / 无表情/无视线 | 保留可选             |
| `dm_125` | 站立待机<br>Standing idle                                  | idle       | 14.00s | 51 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_126` | 站立待机<br>Standing idle                                  | idle       | 14.13s | 51 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_128` | 放松站立待机<br>Standing relaxed                           | idle       | 14.50s | 51 骨 / 30 指骨 / 无表情/无视线 | 保留可选             |
| `dm_134` | 看手表<br>Looking at watch                                 | oneshot    | 13.63s | 48 骨 / 27 指骨 / 无表情/无视线 | 保留可选             |
| `dm_135` | 查看时间<br>Checking time                                  | oneshot    | 13.21s | 48 骨 / 27 指骨 / 无表情/无视线 | 保留可选             |
| `dm_139` | 开心站立待机<br>Happy standing idle                        | idle       | 13.21s | 45 骨 / 24 指骨 / 无表情/无视线 | 保留可选             |

## 判定口径

- **A**：在默认 VRM 上技术通过，骨骼覆盖和动作语义都适合当前桌宠场景。
- **B**：技术兼容且语义可用，但缺少手指/表情/LookAt，或持续时间偏长，需要 Runtime V5 叠加反馈和安全打断。
- **C**：可以播放，但重复、语义不明、过长或不适合高频桌宠调度。
- **D**：依赖当前不存在的道具/场景，或内容定位不适合内置包。
- **保留核心**：进入默认预加载或高频调度集合。
- **必须保留依赖**：被 recovery 派生动作引用；替换依赖关系前不能删除。
- **保留可选**：移入按需下载/默认不调度的动作包，不占核心资源和决策空间。
- **建议删除**：从内置目录和同步源中移除；如确有用户需求，应由用户自定义导入承担。

技术兼容结论来自现有集成测试：全部条目均可重定向并在默认 VRM 的起点、中点、末点采样，未出现 NaN 或无效四元数。它证明“能播”，不等于已逐帧人工确认视觉质量。产品适配结论依据动作语义、时长、通道覆盖、依赖场景、重复度和当前桌宠交互目标；最终删除前仍应在 Motion Lab 对拟保留核心集合做一次人工视觉签收。

## 逐项清单

|   # | 动作 ID / 中文名                                                                     |   时长 | 家族               | 具体动作与用途           | 覆盖                            | 适配 | 建议         | 依据                                                                               |
| --: | ------------------------------------------------------------------------------------ | -----: | ------------------ | ------------------------ | ------------------------------- | :--: | ------------ | ---------------------------------------------------------------------------------- |
|   1 | `builtin.clawatar.angry-gesture.94a2a266`<br>生气手势                                |  1.96s | reaction/action    | 短促的生气抗议手势       | 22 骨 / 0 指骨 / 无表情/无视线  |  B   | 保留核心     | 语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现                     |
|   2 | `builtin.clawatar.angry-shhh.a1c68de4`<br>生气地示意安静                             |  5.75s | reaction/action    | 生气地示意安静           | 48 骨 / 29 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_52 标为 reaction；先作为可选动作进行视觉签收                |
|   3 | `builtin.clawatar.annoyed-head-shake.5975c067`<br>烦恼摇头                           |  2.29s | reaction/action    | 不耐烦地摇头             | 22 骨 / 0 指骨 / 无表情/无视线  |  B   | 保留核心     | 语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现                     |
|   4 | `builtin.clawatar.arms-crossed-x-say-no.fcae82b1`<br>双臂交叉表示拒绝                |  4.88s | reaction/action    | 双臂交叉表示拒绝         | 43 骨 / 24 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_27 标为 reaction；先作为可选动作进行视觉签收                |
|   5 | `builtin.clawatar.bboy-hip-hop-move.105bf175`<br>B-boy 嘻哈动作                      |  2.17s | gesture/action     | 短促的 B-boy 嘻哈招式    | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可播放但缺少表情、LookAt 和多数手指细节，适合作为扩展动作                          |
|   6 | `builtin.clawatar.belly-dance.a6717a4a`<br>肚皮舞                                    | 19.63s | performance/action | 长段肚皮舞表演           | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可作为低频娱乐动作，但不应进入默认高频调度                                         |
|   7 | `builtin.clawatar.cat-pose.830821bb`<br>猫咪姿势                                     |  5.38s | reaction/action    | 猫咪姿势                 | 45 骨 / 26 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_48 标为 reaction；先作为可选动作进行视觉签收                |
|   8 | `builtin.clawatar.cat-pose.bb76cce8`<br>猫咪姿势                                     |  5.92s | reaction/action    | 猫咪姿势                 | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_43 标为 reaction；先作为可选动作进行视觉签收                |
|   9 | `builtin.clawatar.checking-time.88a35d18`<br>查看时间                                | 13.21s | gesture/action     | 查看时间                 | 48 骨 / 27 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_135 标为 oneshot；先作为可选动作进行视觉签收                |
|  10 | `builtin.clawatar.cheer-encourage.746ac7af`<br>加油鼓励                              |  3.58s | reaction/action    | 加油鼓励                 | 46 骨 / 27 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_28 标为 reaction；先作为可选动作进行视觉签收                |
|  11 | `builtin.clawatar.cheer-fighting.2c4d631b`<br>挥拳加油                               |  3.21s | reaction/action    | 挥拳加油                 | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_45 标为 reaction；先作为可选动作进行视觉签收                |
|  12 | `builtin.clawatar.chicken-dance.3ebf03fa`<br>小鸡舞                                  |  4.75s | performance/action | 小鸡舞式的夸张娱乐动作   | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可作为低频娱乐动作，但不应进入默认高频调度                                         |
|  13 | `builtin.clawatar.chill-pose.ec8bffe6`<br>放松姿势                                   | 12.58s | idle/base          | 放松姿势                 | 37 骨 / 18 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_101 标为 idle；先作为可选动作进行视觉签收                   |
|  14 | `builtin.clawatar.comforting-tired-sleepy-user.1e149855`<br>安慰疲惫的用户           |  4.50s | reaction/action    | 安慰疲惫的用户           | 19 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_20 标为 reaction；先作为可选动作进行视觉签收                |
|  15 | `builtin.clawatar.cool-pose.8248deca`<br>酷系姿势                                    |  6.04s | idle/base          | 酷系姿势                 | 43 骨 / 24 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_33 标为 idle；先作为可选动作进行视觉签收                    |
|  16 | `builtin.clawatar.cool-standby-pose.c7555d77`<br>酷系待机                            |  6.54s | idle/base          | 酷系待机                 | 44 骨 / 25 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_23 标为 idle；先作为可选动作进行视觉签收                    |
|  17 | `builtin.clawatar.cute-jumping.98827d44`<br>可爱跳跃                                 |  6.21s | gesture/action     | 可爱跳跃                 | 19 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_32 标为 oneshot；先作为可选动作进行视觉签收                 |
|  18 | `builtin.clawatar.cute-peace-sign.af51ab80`<br>可爱胜利手势                          |  3.21s | reaction/action    | 可爱胜利手势             | 41 骨 / 22 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_26 标为 reaction；先作为可选动作进行视觉签收                |
|  19 | `builtin.clawatar.cute-pose.841421dd`<br>可爱姿势                                    |  4.96s | reaction/action    | 可爱姿势                 | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_56 标为 reaction；先作为可选动作进行视觉签收                |
|  20 | `builtin.clawatar.cute-standby.dda9b85c`<br>可爱待机                                 |  6.54s | idle/base          | 可爱待机                 | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_24 标为 idle；先作为可选动作进行视觉签收                    |
|  21 | `builtin.clawatar.dog-pose.7588810e`<br>小狗姿势                                     |  4.42s | reaction/action    | 小狗姿势                 | 19 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_47 标为 reaction；先作为可选动作进行视觉签收                |
|  22 | `builtin.clawatar.encouraging-user.80244687`<br>鼓励用户                             |  5.63s | reaction/action    | 鼓励用户                 | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_2 标为 reaction；先作为可选动作进行视觉签收                 |
|  23 | `builtin.clawatar.energetic-cheering.1301cacd`<br>活力加油                           |  4.79s | reaction/action    | 活力加油                 | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_53 标为 reaction；先作为可选动作进行视觉签收                |
|  24 | `builtin.clawatar.exercising-standby.5aff055a`<br>运动式待机                         |  6.50s | idle/base          | 运动式待机               | 45 骨 / 26 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_63 标为 idle；先作为可选动作进行视觉签收                    |
|  25 | `builtin.clawatar.fidgeting-fingers-made-a-mistake.ba2ab6fd`<br>犯错后不安地摆弄手指 |  6.50s | reaction/action    | 犯错后不安地摆弄手指     | 39 骨 / 20 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_40 标为 reaction；先作为可选动作进行视觉签收                |
|  26 | `builtin.clawatar.finger-to-lips-shhh.6d59a1e2`<br>手指抵唇示意安静                  |  6.50s | reaction/action    | 手指抵唇示意安静         | 46 骨 / 27 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_42 标为 reaction；先作为可选动作进行视觉签收                |
|  27 | `builtin.clawatar.greeting-user.c8cf7fe0`<br>问候用户                                |  4.50s | reaction/action    | 问候用户                 | 19 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_4 标为 reaction；先作为可选动作进行视觉签收                 |
|  28 | `builtin.clawatar.happy-hand-gesture.79fd8cf9`<br>开心手势                           |  2.63s | gesture/action     | 开心时的短手势           | 22 骨 / 0 指骨 / 无表情/无视线  |  B   | 保留核心     | 语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现                     |
|  29 | `builtin.clawatar.happy-small-jump.de2edb47`<br>开心小跳                             |  7.38s | gesture/action     | 开心小跳                 | 19 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_19 标为 oneshot；先作为可选动作进行视觉签收                 |
|  30 | `builtin.clawatar.happy-standing-idle.5baeb837`<br>开心站立待机                      | 13.21s | idle/base          | 开心站立待机             | 45 骨 / 24 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_139 标为 idle；先作为可选动作进行视觉签收                   |
|  31 | `builtin.clawatar.head-nod-yes.e6be0649`<br>点头同意                                 |  2.33s | gesture/action     | 点头表示同意             | 22 骨 / 0 指骨 / 无表情/无视线  |  B   | 保留核心     | 语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现                     |
|  32 | `builtin.clawatar.heart-hands.70ef82cb`<br>双手比心                                  |  3.33s | reaction/action    | 双手比心                 | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_29 标为 reaction；先作为可选动作进行视觉签收                |
|  33 | `builtin.clawatar.jazz-dancing.1974212c`<br>爵士舞                                   |  5.42s | performance/action | 短段爵士舞表演           | 21 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可作为低频娱乐动作，但不应进入默认高频调度                                         |
|  34 | `builtin.clawatar.joyful-jump.db51c1ba`<br>开心跳跃                                  |  1.83s | performance/action | 开心地跳起庆祝           | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可作为低频娱乐动作，但不应进入默认高频调度                                         |
|  35 | `builtin.clawatar.little-tiger-cat-pose.06743d40`<br>小老虎或猫咪姿势                |  8.00s | reaction/action    | 小老虎或猫咪姿势         | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_57 标为 reaction；先作为可选动作进行视觉签收                |
|  36 | `builtin.clawatar.looking-at-watch.8791712c`<br>看手表                               | 13.63s | gesture/action     | 看手表                   | 48 骨 / 27 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_134 标为 oneshot；先作为可选动作进行视觉签收                |
|  37 | `builtin.clawatar.loser.d10cb204`<br>失败手势                                        |  3.25s | gesture/action     | 表示失败或嘲讽的手势     | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可播放但缺少表情、LookAt 和多数手指细节，适合作为扩展动作                          |
|  38 | `builtin.clawatar.macarena-dance.2726687d`<br>玛卡莲娜舞                             |  8.21s | performance/action | 玛卡莲娜舞表演           | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可作为低频娱乐动作，但不应进入默认高频调度                                         |
|  39 | `builtin.clawatar.neck-stretching.5475198c`<br>颈部拉伸                              |  2.88s | gesture/action     | 左右活动颈部             | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可播放但缺少表情、LookAt 和多数手指细节，适合作为扩展动作                          |
|  40 | `builtin.clawatar.nice-standing-idle.4c1c21ee`<br>自然站立待机                       | 13.50s | idle/base          | 自然站立待机             | 51 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_123 标为 idle；先作为可选动作进行视觉签收                   |
|  41 | `builtin.clawatar.peace-sign.56c6d5ec`<br>胜利手势                                   |  9.79s | reaction/action    | 胜利手势                 | 19 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_30 标为 reaction；先作为可选动作进行视觉签收                |
|  42 | `builtin.clawatar.peace-sign.e583a161`<br>胜利手势                                   |  4.71s | reaction/action    | 胜利手势                 | 41 骨 / 22 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_44 标为 reaction；先作为可选动作进行视觉签收                |
|  43 | `builtin.clawatar.peace-sign-variant.2687650b`<br>胜利手势变化                       |  7.58s | reaction/action    | 胜利手势变化             | 18 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_31 标为 reaction；先作为可选动作进行视觉签收                |
|  44 | `builtin.clawatar.pointing-directing-thinking.4cba9d93`<br>指向并思考                |  8.17s | reaction/action    | 指向并思考               | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_108 标为 reaction；先作为可选动作进行视觉签收               |
|  45 | `builtin.clawatar.presenting-showing-something.2caea0bb`<br>介绍或展示内容           | 12.88s | speech/speech      | 介绍或展示内容           | 43 骨 / 24 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_90 标为 talking；先作为可选动作进行视觉签收                 |
|  46 | `builtin.clawatar.quick-informal-bow.a83dfc6d`<br>快速随意鞠躬                       |  2.46s | gesture/action     | 快速随意鞠躬             | 22 骨 / 0 指骨 / 无表情/无视线  |  B   | 保留核心     | 语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现                     |
|  47 | `builtin.clawatar.rejected.5e24587d`<br>被拒绝                                       |  4.63s | reaction/action    | 被拒绝后的失落反应       | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可播放但缺少表情、LookAt 和多数手指细节，适合作为扩展动作                          |
|  48 | `builtin.clawatar.relieved-sigh.5ccf9e32`<br>放松叹气                                |  2.71s | reaction/action    | 放松并叹气               | 22 骨 / 0 指骨 / 无表情/无视线  |  B   | 保留核心     | 语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现                     |
|  49 | `builtin.clawatar.rumba-dancing.34f7ab39`<br>伦巴舞                                  |  2.33s | performance/action | 短段伦巴舞表演           | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可作为低频娱乐动作，但不应进入默认高频调度                                         |
|  50 | `builtin.clawatar.salute-greeting.86b9cc01`<br>敬礼问候                              |  3.46s | reaction/action    | 敬礼问候                 | 17 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_10 标为 reaction；先作为可选动作进行视觉签收                |
|  51 | `builtin.clawatar.shaking-head-no.2994e837`<br>摇头拒绝                              |  1.63s | gesture/action     | 快速摇头表示否定         | 22 骨 / 0 指骨 / 无表情/无视线  |  B   | 保留核心     | 语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现                     |
|  52 | `builtin.clawatar.shrugging.d548d2e9`<br>耸肩                                        |  1.88s | gesture/action     | 耸肩表示不知道或无奈     | 20 骨 / 0 指骨 / 无表情/无视线  |  B   | 保留核心     | 语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现                     |
|  53 | `builtin.clawatar.shy-hands-on-chest.ff659905`<br>双手放胸前的害羞反应               |  6.54s | reaction/action    | 双手放胸前的害羞反应     | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_51 标为 reaction；先作为可选动作进行视觉签收                |
|  54 | `builtin.clawatar.side-blow-kiss.a5ef7a8c`<br>侧身飞吻                               |  6.17s | reaction/action    | 侧身飞吻                 | 19 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_21 标为 reaction；先作为可选动作进行视觉签收                |
|  55 | `builtin.clawatar.silly-dancing.7fc7963b`<br>搞怪舞蹈                                |  3.83s | performance/action | 短段搞怪舞蹈             | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可作为低频娱乐动作，但不应进入默认高频调度                                         |
|  56 | `builtin.clawatar.singing-pose.68c3955f`<br>唱歌姿势                                 | 22.67s | reaction/action    | 唱歌姿势                 | 34 骨 / 15 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_97 标为 reaction；先作为可选动作进行视觉签收                |
|  57 | `builtin.clawatar.sitting-stretch.381ac85b`<br>坐姿伸展                              | 15.29s | idle/base          | 坐姿伸展                 | 21 骨 / 0 指骨 / 无表情/无视线  |  D   | 建议删除     | 上游明确为坐姿动作，当前透明桌面场景没有座椅和坐姿锚点                             |
|  58 | `builtin.clawatar.sitting-stretch.eb9ef583`<br>坐姿伸展                              | 20.29s | idle/base          | 坐姿伸展                 | 21 骨 / 0 指骨 / 无表情/无视线  |  D   | 建议删除     | 上游明确为坐姿动作，当前透明桌面场景没有座椅和坐姿锚点                             |
|  59 | `builtin.clawatar.slightly-depressed-tired-sleepy.8ad90292`<br>轻微低落或疲惫待机    |  7.38s | idle/base          | 轻微低落或疲惫待机       | 19 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_17 标为 idle-night；先作为可选动作进行视觉签收              |
|  60 | `builtin.clawatar.slower-salute-greeting.5d19a4bb`<br>缓慢敬礼问候                   |  3.67s | reaction/action    | 缓慢敬礼问候             | 23 骨 / 4 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_11 标为 reaction；先作为可选动作进行视觉签收                |
|  61 | `builtin.clawatar.standing-idle.2bd79d7d`<br>站立待机                                | 15.25s | idle/base          | 站立待机                 | 45 骨 / 24 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_120 标为 idle；先作为可选动作进行视觉签收                   |
|  62 | `builtin.clawatar.standing-idle.332ba299`<br>站立待机                                | 16.46s | idle/base          | 站立待机                 | 47 骨 / 26 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_122 标为 idle；先作为可选动作进行视觉签收                   |
|  63 | `builtin.clawatar.standing-idle.70db7547`<br>站立待机                                | 14.00s | idle/base          | 站立待机                 | 51 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_125 标为 idle；先作为可选动作进行视觉签收                   |
|  64 | `builtin.clawatar.standing-idle.a5bb3c9e`<br>站立待机                                | 14.13s | idle/base          | 站立待机                 | 51 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_126 标为 idle；先作为可选动作进行视觉签收                   |
|  65 | `builtin.clawatar.standing-idle-variant.0dcf21ad`<br>站立待机变化                    | 16.29s | idle/base          | 站立待机变化             | 45 骨 / 24 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_124 标为 idle；先作为可选动作进行视觉签收                   |
|  66 | `builtin.clawatar.standing-relaxed.046fc19a`<br>放松站立待机                         | 14.50s | idle/base          | 放松站立待机             | 51 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_128 标为 idle；先作为可选动作进行视觉签收                   |
|  67 | `builtin.clawatar.standing-relaxed.bde24180`<br>放松站立待机                         | 15.96s | idle/base          | 放松站立待机             | 51 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_121 标为 idle；先作为可选动作进行视觉签收                   |
|  68 | `builtin.clawatar.standing-stretch.f14c405b`<br>站立伸展                             | 12.46s | idle/base          | 站立伸展                 | 21 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_86 标为 idle；先作为可选动作进行视觉签收                    |
|  69 | `builtin.clawatar.stretching-idle.00dc0b1c`<br>伸展待机                              | 13.63s | idle/base          | 伸展待机                 | 21 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_82 标为 idle；先作为可选动作进行视觉签收                    |
|  70 | `builtin.clawatar.swing-dancing-3.432b59d5`<br>摇摆舞 3                              |  2.46s | performance/action | 短段摇摆舞表演           | 22 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | 可作为低频娱乐动作，但不应进入默认高频调度                                         |
|  71 | `builtin.clawatar.user-assigns-task-received.19e6db25`<br>确认收到用户任务           |  3.17s | reaction/action    | 确认收到用户任务         | 20 骨 / 1 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_12 标为 reaction；先作为可选动作进行视觉签收                |
|  72 | `builtin.clawatar.user-bullies-whiny-protest.b4700b80`<br>被欺负后的委屈抗议         |  3.58s | reaction/action    | 被欺负后的委屈抗议       | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_9 标为 reaction；先作为可选动作进行视觉签收                 |
|  73 | `builtin.clawatar.user-pokes-or-teases.ebaf4770`<br>被戳或逗弄后的反应               |  3.25s | reaction/action    | 被戳或逗弄后的反应       | 49 骨 / 30 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_8 标为 reaction；先作为可选动作进行视觉签收                 |
|  74 | `builtin.clawatar.very-cute-greeting.1aac931c`<br>可爱问候                           |  7.46s | reaction/action    | 可爱问候                 | 46 骨 / 27 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_39 标为 reaction；先作为可选动作进行视觉签收                |
|  75 | `builtin.clawatar.warm-up-idle.12aad66a`<br>热身待机                                 | 14.33s | idle/base          | 热身待机                 | 21 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_88 标为 idle；先作为可选动作进行视觉签收                    |
|  76 | `builtin.clawatar.warm-up-idle.6bd5f7f7`<br>热身待机                                 | 15.00s | idle/base          | 热身待机                 | 21 骨 / 0 指骨 / 无表情/无视线  |  C   | 保留可选     | Clawatar 固定提交将 dm_89 标为 idle；先作为可选动作进行视觉签收                    |
|  77 | `builtin.clawatar.waving.8887b88a`<br>挥手                                           |  2.92s | gesture/action     | 挥手问候或告别           | 22 骨 / 0 指骨 / 无表情/无视线  |  B   | 保留核心     | 语义明确且足够短；身体兼容，但需由运行时表情和 LookAt 补足表现                     |
|  78 | `builtin.clawatar.yawning-stretching-idle.0bf6ab00`<br>打哈欠并伸展的待机            |  8.08s | idle/base          | 打哈欠并伸展的待机       | 34 骨 / 15 指骨 / 无表情/无视线 |  C   | 保留可选     | Clawatar 固定提交将 dm_22 标为 idle；先作为可选动作进行视觉签收                    |
|  79 | `builtin.hachimi.action-recover-to-idle.v1`<br>动作恢复待机                          |  0.26s | recovery/action    | 动作被打断后回到中性待机 | 49 骨 / 28 指骨 / 表情/视线     |  B   | 必须保留依赖 | V5 action_recover_to_idle 运行时片段，依赖 `builtin.openmaiwaifu.waiting.3b2e83e2` |
|  80 | `builtin.openmaiwaifu.appearing.b4240ae2`<br>登场                                    |  4.50s | gesture/action     | 角色登场亮相             | 51 骨 / 30 指骨 / 表情/视线     |  A   | 保留核心     | 覆盖手指/表情，适合高频待机或直接交互                                              |
|  81 | `builtin.openmaiwaifu.cool-liked.78dd992c`<br>酷系开心回应                           |  7.60s | gesture/action     | 酷系角色被喜欢后的回应   | 51 骨 / 30 指骨 / 表情/无视线   |  A   | 保留可选     | 骨骼、手指及多数面部通道覆盖完整，适合作为人格扩展                                 |
|  82 | `builtin.openmaiwaifu.cool-waiting.b39d588f`<br>酷系待机                             | 20.50s | idle/base          | 酷系自然待机循环         | 51 骨 / 30 指骨 / 表情/视线     |  A   | 保留可选     | 高覆盖人格化待机，按模型气质启用，避免全部同时随机                                 |
|  83 | `builtin.openmaiwaifu.flamboyant-liked.2936f285`<br>华丽开心回应                     |  5.47s | gesture/action     | 华丽型开心回应           | 51 骨 / 30 指骨 / 表情/视线     |  A   | 保留可选     | 骨骼、手指及多数面部通道覆盖完整，适合作为人格扩展                                 |
|  84 | `builtin.openmaiwaifu.gentleman-liked.8f4b60b3`<br>绅士开心回应                      |  6.70s | gesture/action     | 绅士型开心回应           | 51 骨 / 30 指骨 / 表情/无视线   |  A   | 保留可选     | 骨骼、手指及多数面部通道覆盖完整，适合作为人格扩展                                 |
|  85 | `builtin.openmaiwaifu.gentleman-waiting.1be8a0f6`<br>绅士待机                        | 18.20s | idle/base          | 绅士型自然待机循环       | 51 骨 / 30 指骨 / 表情/视线     |  A   | 保留可选     | 高覆盖人格化待机，按模型气质启用，避免全部同时随机                                 |
|  86 | `builtin.openmaiwaifu.happy.ba2b5f00`<br>开心                                        |  6.90s | gesture/action     | 完整的开心全身反应       | 51 骨 / 30 指骨 / 表情/视线     |  A   | 保留核心     | 覆盖手指/表情，适合高频待机或直接交互                                              |
|  87 | `builtin.openmaiwaifu.ladylike-liked.3ce5ab7d`<br>淑女开心回应                       |  7.73s | gesture/action     | 淑女型开心回应           | 51 骨 / 30 指骨 / 表情/视线     |  A   | 保留可选     | 骨骼、手指及多数面部通道覆盖完整，适合作为人格扩展                                 |
|  88 | `builtin.openmaiwaifu.ladylike-waiting.3ee058e2`<br>淑女待机                         | 13.67s | idle/base          | 淑女型自然待机循环       | 51 骨 / 30 指骨 / 表情/视线     |  A   | 保留可选     | 高覆盖人格化待机，按模型气质启用，避免全部同时随机                                 |
|  89 | `builtin.openmaiwaifu.laughing.893ae64f`<br>大笑                                     |  7.87s | gesture/action     | 带表情和身体动作的大笑   | 51 骨 / 30 指骨 / 表情/视线     |  A   | 保留核心     | 覆盖手指/表情，适合高频待机或直接交互                                              |
|  90 | `builtin.openmaiwaifu.liked.e15f48cd`<br>开心回应                                    | 11.17s | gesture/action     | 较长的通用开心回应       | 51 骨 / 30 指骨 / 表情/视线     |  B   | 保留可选     | 技术覆盖完整但反馈过长，应低频使用并允许 safe-point 打断                           |
|  91 | `builtin.openmaiwaifu.manly-appearing.63ac2a26`<br>帅气登场                          |  3.33s | gesture/action     | 帅气风格的登场亮相       | 51 骨 / 30 指骨 / 表情/视线     |  A   | 保留可选     | 骨骼、手指及多数面部通道覆盖完整，适合作为人格扩展                                 |
|  92 | `builtin.openmaiwaifu.stretching.5d4839c1`<br>伸展                                   | 13.70s | gesture/action     | 较完整的全身伸展         | 51 骨 / 30 指骨 / 表情/视线     |  B   | 保留可选     | 技术覆盖完整但反馈过长，应低频使用并允许 safe-point 打断                           |
|  93 | `builtin.openmaiwaifu.waiting.3b2e83e2`<br>等待                                      | 14.50s | idle/base          | 通用自然等待循环         | 49 骨 / 28 指骨 / 表情/视线     |  A   | 保留核心     | 覆盖手指/表情，适合高频待机或直接交互                                              |

## 对动作连贯性的直接影响

清理目录不能替代 Runtime V5 的 TransitionPlanner 和惯性化，但已经消除了错误切换来源：BehaviorScheduler 不再把 DM 的对话、反应和待机当普通舞蹈，也不会选择已移除的长动作、空气倚靠或无手机通话。核心集合应按以下方式使用：

| 产品意图  | 首选动作                                                 | 调度说明                                           |
| --------- | -------------------------------------------------------- | -------------------------------------------------- |
| 首次显示  | `appearing` 覆盖在 `waiting` 上                          | 两个动作都准备完成且首帧已采样后才显示模型         |
| 默认 idle | `waiting`                                                | 唯一常驻 base；一次性动作淡出后自然露出该层        |
| 通用沟通  | `waving`、`head nod yes`、`shaking head no`、`shrugging` | 都是短且语义明确的离散动作；同类只保留最新请求     |
| 语音      | LipSync、表情和注视层                                    | 已删除通用 `talking` VRMA，避免全身动作抢占 base   |
| 移动      | 无内置动作                                               | locomotion slot 保留为扩展点，不以待机滑动代替行走 |
| 打断恢复  | `action_recover_to_idle`                                 | 取 `waiting` 前 260ms，之后继续由常驻 base 接管    |

## 缺失的核心 VRMA

当前目录即使按本报告清理后，仍不能单靠现有 VRMA 完成计划中的全部“上手感”。下一批资源应优先补齐：

1. `touch_react_in`、`pet_loop`、`pet_release`：现有 liked 动作都是一次性开心回应，无法表达持续抚摸的输入开始、保持和自然释放。
2. `speech_upper_body_neutral`：新资源应只占上身并可按语音能量连续缩放，避免抢占 waiting 和触摸。
3. 成套 locomotion：只有 walk start/loop/stop、turn 和 recover 都具备独立的预备、制动、重心转移后才重新引入。
4. 真正制作的 `action_recover_to_idle`：当前片段只是 `waiting` 的前 260ms，职责上可用但不是针对任意源姿势设计的恢复动作。
5. `idle_micro_shift` 与 `interaction_recover`：用于长时间桌面停留中的小幅重心变化，以及触摸/语音结束后的短收势，减少每次都回到同一明显关键帧。

在这些专用资源完成前，动作恢复片段必须保留，但适配等级按 B 处理：技术正确、能防止 T Pose，不代表重心、脚接触和收势已经达到最终视觉标准。

## 后续清理顺序

1. 在 Motion Lab 人工确认 `dm_85`、`dm_87`；确认后加入同步排除并删除无引用 blob。
2. 对其余 55 个 DM 动作按 reaction、idle、speech、gesture 分批预览，只将明显优于现有核心动作的少量条目升级为核心。
3. 将保留可选项拆为独立动作包，默认不预加载、不参与自主随机调度，仅由明确命令触发。
4. 对最终核心集合执行 `idle → action → interaction → recovery → idle` 矩阵，并人工检查裙摆、长发、手指穿插、脚底漂移和镜头出界。

## 对当前模型的总体判断

OpenMaiWaifu 系列通常覆盖 49–52 根 humanoid 骨骼、28–30 根手指骨，并经常携带表情和 LookAt，和当前默认二次元 VRM 的表现能力更匹配。Clawatar 的原命名动作大多只有约 20–22 根身体骨骼；DM Motionpack 覆盖范围为 17–51 根骨骼，其中一部分带手指轨道，但两组都没有表情和 LookAt，仍需 Runtime V5 的表情、注视与交互反馈层补足。

不同用户 VRM 的体型、裙装、长发和骨骼比例仍可能改变结果，因此“A”不是对所有模型的永久保证。建议将模型签名维度的 Motion Lab 指标保留为资源准入条件，而不是把默认模型的一次通过扩展成全模型结论。
