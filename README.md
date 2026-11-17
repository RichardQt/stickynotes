# 便签 Sticky Notes

Windows 便签的桌面伴侣应用，使用 Rust + GPUI 编写。直接读写系统便签的 `plum.sqlite`，与原生便签数据保持同步。

> 🚧 **项目正在持续开发中**，功能、界面和数据结构都可能随时调整。

## 功能

- 读取 / 写入 Windows 便签数据库（`plum.sqlite`）
- 标签分类、置顶、全文搜索
- 8 种便签纸主题
- Markdown 编辑器：标题与正文分离、图标格式工具条
- 外部修改自动同步（轮询 plum.sqlite 与 WAL）

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
