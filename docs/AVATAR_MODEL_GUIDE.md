# 3D 角色模型指南

Hachimi Avatar Motion Runtime V4 的模型导入只接受 `.vrm`，支持 VRM 0.x 与 VRM 1.0。普通 `.glb` 不进入桌宠运行时。身体动作来自内置或用户导入的正式 VRMA 1.0，并动态重定向到当前 Runtime Ready 模型。

## Runtime Ready 要求

导入对话框会逐项检查：

- 完整的可渲染蒙皮网格和有限边界；
- VRM 核心标准 Humanoid（胸骨、眼球、脚趾和手指为可选降级项）；
- 可检测的表情、LookAt、MToon、SpringBone 与 Collider 能力；
- `aa` 或完整 aa/ih/ou/ee/oh 决定 `jaw`/`five_viseme`；没有嘴型时 Pet 自动禁用语音但保留文字；
- 自包含资源、合法 BufferView、最多四个有效蒙皮权重；
- 文件、三角形、节点、关节、材质、纹理尺寸和估算显存均在预算内。

只有所有必需项通过时才签发十分钟、单次使用且绑定 Workbench Client ID 的导入 Token。提交时 Rust 会重新验证文件大小、修改时间和 SHA-256，WebView 不会得到任意本地路径。

## 推荐来源

- [VRoid Hub](https://hub.vroid.com/en)：筛选允许下载的 VRM 角色。
- [BOOTH 免费 VRM 搜索](https://booth.pm/en/search/VRM?max_price=0)：每个资源的授权不同，必须单独确认。
- [VRoid Studio](https://vroid.com/en/studio)：免费创建并导出自己的 VRM，最适合作为可控角色来源。
- [VRM Add-on for Blender](https://github.com/saturday06/VRM-Addon-for-Blender)：为已有模型完成 Humanoid、表情、LookAt、MToon 与 SpringBone 配置并导出 VRM。

Sketchfab 下载的 GLB 不能直接导入 Runtime V4。将 `.glb` 后缀改成 `.vrm` 也不会生成 VRM 元数据；必须在 Blender、Unity 或 VRoid 工具链中完成骨骼、蒙皮、表情、材质和二级物理配置，再导出为 VRM。

## 授权检查

用户本地导入不代表模型可以随安装包重新发布。将模型作为 Hachimi 默认资源前，必须确认商业使用、修改、格式转换、署名和再分发权限；免费或零元资源不自动包含这些权利。

动作可在 Workbench 的“动作库”中检索、预览和绑定；内置动作不可删除，用户动作删除时会原子清理相关互动绑定。
