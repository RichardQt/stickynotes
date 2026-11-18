# 便签 Sticky Notes

Windows 便签的桌面伴侣应用，使用 Rust + GPUI 编写。直接读写系统便签的 `plum.sqlite`，与原生便签数据保持同步。

> 🚧 **项目正在持续开发中**，功能、界面和数据结构都可能随时调整。

## 功能

- 读取 / 写入 Windows 便签数据库（`plum.sqlite`）
- 标签分类、置顶、全文搜索
- 8 种便签纸主题
- Markdown 编辑器：标题与正文分离、图标格式工具条
- WebDAV 同步：每条便签保存为 `<id>.md`（YAML Frontmatter + Markdown 正文），默认开启自动同步
- 外部修改自动同步（轮询 plum.sqlite 与 WAL）

## WebDAV 同步

侧边栏底部的齿轮按钮打开设置：

- **自动同步（WebDAV）**：默认开启。编辑后约 2.5 秒上传；启动时和后台每分钟拉取合并，按 `updated_at` 最新者胜出。未填写服务器地址时静默跳过。
- **同步微软账户（Windows 便签）**：默认关闭。开启后才会跟随原生便签应用及其云端的变化。
- 表单：服务器地址、用户名、密码（可切换显示）、远端目录（默认 `/StickyNotes/`，不存在时自动 MKCOL 创建）。
- **测试连接**：非空校验 + Basic Auth 的 `PROPFIND` 请求，内联显示结果。

配置保存在 `%LOCALAPPDATA%\stickynotes\config.json`（密码为明文存储，建议使用应用专用密码）。远端文件可直接用任意 Markdown 编辑器打开。

远端文件开头的 YAML Frontmatter（`---` 包围的元数据块）用于记录 `id`、颜色、标签、置顶和修改时间，是同步冲突判断的依据。Obsidian、VS Code、GitHub、Typora 等会在预览中自动隐藏它；如果你的查看器不支持 Frontmatter，元数据会作为普通文本显示，此时请用上述编辑器打开或查看原始文本——便签正文本身不受影响。

## 运行

```bash
cargo run -p stickynotes-app
```

需要 Rust 工具链，支持 Windows 10 / 11。

## 目录结构

| 路径 | 说明 |
| --- | --- |
| `crates/app` | 桌面 UI（GPUI） |
| `crates/core` | plum.sqlite、标签库、同步逻辑 |
| `crates/vendor/markdown-editor` | Markdown 编辑器组件 |

## 说明

- 便签内容属于个人隐私：`target/`、`backups/`、截图、日志和编译产物均已在 `.gitignore` 中排除，不会进入仓库，也不影响正常推送。
- 应用会直接修改系统便签数据库，重要数据请先自行备份。
