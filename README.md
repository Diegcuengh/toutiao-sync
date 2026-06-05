# 今日头条收藏同步桌面端

目标：
- Rust + Vue + Tauri
- 增量同步今日头条“收藏 / 喜欢”
- 点击按钮触发同步
- 保留同步历史与同步日志
- 支持本地内容搜索
- 视频、文章保存到本地

当前实现先把三层链路打通：

1. `src-tauri/`：本地数据库、同步历史、同步日志、搜索、后台同步调度
2. `src/`：桌面 UI，包含同步按钮、会话历史、日志查看、本地搜索
3. `scripts/toutiao_sync.js`：浏览器抓取与本地下载脚本，复用旧项目里验证过的思路：
   - Chrome CDP 连接
   - 登录态复用
   - 增量去重
   - 文章/视频本地落盘

## 复用旧代码的结论

用户提到的路径里，实际可参考的是：

- `C:\Users\Alex\WorkBuddy\20260408131321`

本项目没有直接照搬旧 Node 项目，而是迁移了这些设计：

- `common/browser.js` 的 Chrome/CDP 连接思路
- `common/resource_manager.js` 的资源去重与下载记录思路
- `scrape.js` 的任务会话、断点续跑、增量调度思路

## 目录

```text
src/                     Vue 界面
src-tauri/               Tauri + Rust 后端
scripts/toutiao_sync.js  今日头条同步脚本
dist/                    前端构建产物
```

运行后的本地数据默认放在系统 `app_data_dir/toutiao-sync/` 下，包括：
- `app.db`
- `downloads/`
- `jobs/`

## 运行

先安装依赖：

```powershell
npm.cmd install
```

开发模式：

```powershell
npm.cmd run tauri dev
```

打包：

```powershell
npm.cmd run build
cargo tauri build
```

## 使用方式

1. 启动应用
2. 确认本机已安装 Node.js，并且 `node -v` 可用
3. 在 Chrome 手动登录今日头条
4. 打开“收藏”或“喜欢”列表页
5. 回到桌面端，点击“开始增量同步”
6. 在“同步历史 / 同步日志”里看过程
7. 在“本地内容”里搜索，并打开本地目录

## 当前边界

当前已经具备：
- 本地 SQLite 数据模型
- 增量同步与去重
- 同步历史与事件日志
- 全文搜索
- 文章 HTML / JSON 落地
- 视频下载到本地
- 打包后的脚本资源随应用分发

仍需真实页面实测继续校准的部分：
- 今日头条列表页 DOM 选择器
- 视频详情页、微头条详情页字段差异
- 登录失效时的交互体验
- 打包版对本机 Node.js 的依赖提示
