# Aseprite MCP

让 AI agent（Claude Code 等 MCP Client）直接在**正在运行的 Aseprite** 里画图、改图层、检查 Sprite。改动实时可见，一次调用就是一步撤销（Ctrl+Z）。

安装扩展后，每次打开 Aseprite 都会自动拉起随扩展打包的 MCP Server，MCP Client 连接 `http://127.0.0.1:48653/mcp` 即可使用。

> [!NOTE]
> 仅支持 Windows x64，Aseprite 最低版本 **1.3.17**（Steam 当前版本即可）。

## 安装

### 1. 安装扩展

1. 从 [Releases](https://github.com/Insulinocytus/aseprite-mcp/releases) 下载 `aseprite-mcp-<版本>.aseprite-extension`。
2. 双击该文件，按提示安装，然后重启 Aseprite。

### 2. 授权

首次启动时 Aseprite 会弹出一个授权窗口，请求运行 Aseprite MCP 的 Plugin。选择 **完全信任此脚本**（Give this script full trust）。授权是一次性的，之后不再询问。

### 3. 配置 MCP Client

打开 Aseprite，菜单 **Help → MCP Server…** 可查看 Server 和 Bridge 状态，并复制现成的 MCP Client 配置片段。以 Claude Code 为例：

```sh
claude mcp add --transport http aseprite http://127.0.0.1:48653/mcp
```

其他 MCP Client 请使用 Streamable HTTP 传输，地址同上。端口冲突时，可在该窗口修改端口并点击重启，MCP URL 会随之更新。

### 4. 安装 Skill（可选）

Skill 教 agent 写正确的 Aseprite Lua：

```sh
npx skills add Insulinocytus/aseprite-mcp
```

也可以从 Releases 下载 `aseprite-lua-skill.zip` 手动安装。

## 使用要点

- `run_lua` 执行修改，整次调用是一个名为 "MCP: run_lua" 的撤销步骤；出错时自动回滚。
- `query_lua`、`get_image`、`get_status` 只读，不产生撤销步骤。
- 同时打开两个 Aseprite 时，只有先启动的那个提供 MCP，第二个会收到一次提示。
- Server 只监听 `127.0.0.1`，关闭 Aseprite 后 Server 随即退出。

## 排错

Server 日志在扩展目录下的 `server.log`，通常是：

```
%APPDATA%\Aseprite\extensions\aseprite-mcp\server.log
```

反馈问题时请附上该文件。

## 开发

### 测试

Server seam（需要 Rust，不需要 Aseprite）：

```sh
cargo test --manifest-path server/Cargo.toml
```

Plugin seam（在无界面的 Aseprite 里运行 Lua 测试，需要 Aseprite 与 PowerShell 7）：

```powershell
$env:ASEPRITE = 'C:\path\to\aseprite.exe'
pwsh scripts/test-plugin.ps1
```

### 打包

```powershell
pwsh scripts/package.ps1
```

产物在 `dist/aseprite-mcp-<版本>.aseprite-extension`，版本取自 `plugin/package.json`。

推送 `v<版本>` 标签（须与 `plugin/package.json` 的版本一致）会触发 GitHub Actions：构建 Server、运行 Server seam 测试、打包扩展，并连同 Skill zip 发布 Release。
