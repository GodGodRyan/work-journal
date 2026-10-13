# 每日工作记录记事本（work-journal）

本地优先的桌面工作日志应用：按日历记录每日工作、全文检索、统计趋势、导出 Markdown 报告、一键备份恢复。**无网络、无登录、无云同步**，数据完全保存在本机。

## 技术栈

| 层 | 选型 |
|---|---|
| 桌面壳 | Tauri 2（Rust 后端，无 Electron） |
| 前端 | Vue 3 + TypeScript（全部 `<script setup lang="ts">`）+ Pinia + Vue Router |
| UI | shadcn-vue + Tailwind CSS + lucide 图标（@lucide/vue） |
| 数据库 | SQLite（rusqlite，bundled 编译、启用 FTS5 全文索引） |

所有 CRUD、备份恢复、导出均由 Rust Tauri Command 实现，前端只做视图渲染。

## 功能

- **主页**：日历（有记录的日期高亮圆点）+ 左侧日期导航 + 当日记录列表与编辑器
- **记录编辑**：标题 / 内容 / 状态（待完成、已完成）/ 多标签；500ms 防抖自动保存 + 手动保存（⌘/Ctrl+S 无冲突时自动保存兜底）
- **时间线**：全部记录按日期分组倒序，分页加载
- **搜索**：FTS5 全文搜索（标题 / 内容 / 标签），中文任意子串可命中（单字展开索引），关键词高亮，支持日期范围过滤，300ms 防抖，⌘/Ctrl+F 快速聚焦
- **回收站**：软删除、恢复、彻底删除、清空
- **统计**：本月总数 / 已完成 / 待完成 + 近 6 个月完成趋势柱状图
- **标签**：多标签、按使用次数筛选与管理
- **导出**：Markdown 日报 / 周报 / 月报 / 全部（对话框选择保存位置）
- **备份 / 恢复**：在线备份 API 生成一致性快照；恢复前自动做安全备份，恢复后重建全文索引
- **外观**：亮色 / 暗色 / 跟随系统三主题
- **快捷键**：跟随系统（macOS ⌘ / Windows·Linux Ctrl）——新建记录、全局搜索、手动保存

## 运行与构建

前置要求：Rust（stable）、Node.js 18+（本项目使用 Node 24 LTS）、Tauri 系统依赖（macOS 无额外依赖）。

```bash
# 安装依赖
npm install

# 开发模式（热更新）
npm run tauri dev

# 类型检查
npm run typecheck

# 前端构建
npm run build

# 完整打包（release，产物在 src-tauri/target/release/bundle/）
npm run tauri build

# Rust 单元测试（11 个，覆盖迁移/FTS5/备份恢复/导出）
cd src-tauri && cargo test
```

## 数据库位置与备份

- 数据库文件：**系统应用数据目录**下的 `work-journal.db`
  - macOS：`~/Library/Application Support/com.workjournal.app/work-journal.db`
  - Windows：`C:\Users\<用户>\AppData\Roaming\com.workjournal.app\work-journal.db`
  - Linux：`~/.local/share/com.workjournal.app/work-journal.db`
  - 应用内「设置」页会显示当前实际路径
- 数据库启用 WAL 模式；**备份请使用应用内「备份数据库」功能**（在线备份 API，可安全复制正在使用的库），不要直接拷贝正在运行的数据库文件
- 恢复数据库时，应用会**先自动把当前数据备份**到应用数据目录（`恢复前自动备份-<时间戳>.db`），再执行恢复，误操作可挽回
- 版本迁移：`PRAGMA user_version` 记录 schema 版本，升级时按版本递增执行迁移脚本，历史数据不丢失

## 目录结构

```
├── src/                    # Vue3 前端
│   ├── api/                # Tauri command 封装（前端持久化唯一入口）
│   ├── components/         # 业务组件（日历/编辑器/统计/趋势…）+ ui/（shadcn 基础组件）
│   ├── composables/        # useDate 等组合式函数
│   ├── stores/             # Pinia（app 全局状态）
│   ├── views/              # 五页面：主页/时间线/搜索/回收站/设置
│   └── types/              # 与 Rust 结构体对应的 TS 类型
└── src-tauri/              # Rust 后端
    └── src/
        ├── main.rs         # 入口：初始化数据目录/数据库/注册命令
        ├── commands.rs     # 19 个 Tauri Command：入参校验 + 错误转换
        └── db.rs           # 数据库层：迁移/FTS5/CRUD/备份恢复/Markdown 导出 + 单元测试
```

## 隐私与安全

- 无任何网络请求，数据不出本机
- 搜索查询经参数化绑定 + FTS5 语法转义（防注入）；高亮为前端纯文本渲染（无 v-html 注入面）
- 所有外部输入在 Rust 命令层统一校验（日期格式、状态枚举、字段长度上限）
