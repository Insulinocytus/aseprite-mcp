# aseprite-mcp

让 MCP 客户端驱动正在运行的 Aseprite GUI：插件随 Aseprite 启动本地 MCP 服务，工具调用在用户眼前执行、可撤销。

## Language

**Plugin**:
安装进 Aseprite 的扩展里的 Lua 部分，在运行中的 GUI 内执行工具调用。
_Avoid_: 脚本、客户端、extension script

**Server**:
随扩展打包、由 Plugin 拉起的本地进程，对外提供 MCP 端点。
_Avoid_: 后端、daemon、bridge

**MCP Client**:
连接 Server 的 AI 工具（Claude Code 等）。
_Avoid_: 单说"客户端"（Plugin 在 Bridge 上也是客户端，易混）

**Bridge**:
Plugin 与 Server 之间的 WebSocket 连接，承载工具调用请求与结果。
_Avoid_: socket、link、tunnel

**Bridge Token**:
Plugin 每次启动生成的随机密钥，Server 用它确认 Bridge 对端是拉起自己的那个 Plugin。
_Avoid_: password、API key

**Curated Tool**:
参数固定、行为由本项目预先实现的 MCP 工具（如 `get_image`、`get_status`）。
_Avoid_: 白名单工具、高层工具

**Raw Lua Tool**:
把 MCP Client 提供的任意 Lua 代码交给 Plugin 执行的 MCP 工具。分为写入用、每次调用构成一个撤销步骤的 `run_lua`，和只读、不进撤销历史的 `query_lua`。
_Avoid_: eval、exec

**Aseprite Lua Skill**:
本仓库随附的 agent skill，教 MCP Client 如何写 Aseprite Lua；不含 Raw Lua Tool 自身的调用约定。
_Avoid_: prompt、API 文档
