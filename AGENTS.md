## 项目说明
目前项目还在开发阶段，一些重构和修改不用考虑历史数据的兼容，可以直接清理

### 限制规则

1. 前端文件代码行数不要超过2000行
2. 后端文件代码行数不要超过2000行
3. 前端代码要保证 ui，风格，样式，组件保持一致，前端应该是统一的组件库，样式库这样来保证整体风格，字体，大小，ui，排列一致性
4. 后端 Agent 以 openai/codex 为主要产品与 Runtime 基线，深度对标其统一 Agent、编程、日常办公、桌面控制、浏览器控制、Skills、MCP、Plugins/Connectors、Session/Thread 持久化恢复和 Scheduled Tasks 的产品行为与权限模型；上下文历史消息压缩仅参考 claude-code-best/claude-code 的公开行为；仅在本地常驻 Gateway、Channel 插件与确定性消息路由、Cron/Heartbeat/事件触发、Task ledger、投递和后台任务重启 reconciliation 等能力上深度参考 openclaw/openclaw。所有参考均须固定版本、登记来源，并按当前阶段真实标注实现状态
