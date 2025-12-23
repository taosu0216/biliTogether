# VO Mobile Sync / BiliTools 服务化规划

## 1. 项目背景
- 目标：用手机 Tauri App 与朋友同步观看 B 站或本地视频。房主创建房间，选择播放源，成员输入口令即可同步。
- 当前组件：
  - `vo`（Tauri Android App）：已完成房间 UI、播放源选择、播放器雏形、WebSocket 客户端（`useSyncClient`）。
  - `go server`：提供房间管理、媒体 token、WS 广播，但还不能解析 B 站资源。
  - 第三方参考：《BiliTools》——成熟的 Tauri 桌面客户端，具备登录、解析 DASH/MP4、下载等完整链路，是理想的“桌面端服务”基础。

## 2. 我们需要的能力
1. B 站账号登录 + Cookie 管理（扫码/密码/风控）；
2. 解析 BV/ep 链接，拿到可播放的 DASH/MP4/m3u8 流；
3. 将解析出的流通过本地 HTTP/WS 接口广播给手机端；
4. WebSocket 同步房主状态（play/pause/seek）；
5. （可选）代理/缓存本地文件、字幕等。

## 3. 已有成果
### App 端
- React UI：房间连接、房主/成员角色、播放源 Tab。
- 播放器视图：iframe/`<video>` 占位，展示连接状态、最近一次服务器状态。
- WebSocket Hook：自动构建 `/ws` 连接，支持 host update / member ping。

### Go Server
- 房间管理（tempUser、口令、房主判定）；
- 媒体 token + `/media/{token}` 输出本地文件；
- `/ws` 广播逻辑；
- 单元测试覆盖 `Manager`。

## 4. 决策：用 BiliTools 取代 Go 服务端
理由：
- BiliTools 已经解决登录、DASH 解析、过滤 PCDN 等复杂问题 [BiliTools](https://github.com/btjawa/BiliTools)。
- 桌面 GUI 方便登录/配置；自用时直接运行即可。
- 只需在其 `src-tauri`（Rust）内新增房间/同步 API，就能一站式完成“解析 + 推流 + 同步”。

## 5. 二开 BiliTools 的方案
1. **内置 HTTP/WS 服务**  
   - 选用 `axum`/`warp`，监听固定端口（如 `127.0.0.1:18080`）。
   - API 设计参考现有 Go 服务：
     - `POST /api/room/join`
     - `POST /api/room/update`
     - `POST /api/media/resolve`（BV/ep URL → 解析 → 返回内部的播放 URL）
     - `GET /media/{token}`（输出本地缓存或实时代理）
     - `/ws?room=...`（与 App 复用同一协议）
2. **复用解析逻辑**  
   - 在 Rust 层调用 BiliTools 下载模块，获取 DASH playlist/分片。
   - 选择实时代理（解析后把 upstream 请求转发给 App）或快速缓存（落在本地，供 `/media/{token}` 读取）。
3. **共享登录态**  
   - GUI 完成登录 → 保存 Cookie → HTTP 服务直接读取；不需要手机登录。
4. **App 改动**  
   - base URL 指向桌面端暴露出的地址：本地调试时可用 `127.0.0.1`，上线场景通过 CF Tunnel 等方式获取一个公网 URL，App 只需填入该 URL 即可；协议无需变化。

## 6. 待做事项（按优先级）
1. 在 BiliTools 创建 `vo-sync` 分支，搭建 `axum` 服务骨架；✅ 已完成（端口回退、自建 API）。
2. 把 Go `Manager` 逻辑翻译为 Rust（房间 map、token 等）；✅ 已完成（内存房间/媒体 token、TTL 清理、成员可控开关）。
3. 调用 BiliTools 的解析模块，写出 `/api/media/resolve`；✅ 已接入 BV→playurl 解析，返回 `/media/{token}`，支持本地文件、普通 URL；远程 URL 默认重定向，B 站走代理转发。
4. 将 `vo` App 指向新的服务端地址，验证 join/update/WS；⏳ 正在排查：WS 握手返回 400，服务器已收到 `/api/room/join` 与 `/ws` 请求，但升级后立即关闭且无错误日志，待修。
5. （可选）实现本地文件代理、字幕/弹幕同步、缓存清理；🔜 解析完成后继续。
6. 将 Go 服务端归档，后续仅作为协议参考。🔜

## 7. 备注
- BiliTools GPL-3.0：二开代码需保留开源；自用无妨。
- 如果未来需要公网访问，可在桌面端外再套一层 CF Tunnel 或 Tailscale。
- 当前 App 播放器仍为占位，实现 Tauri WebView + 原生 `<video>` 同步后即可发布 MVP。

## 最新进展与问题 (2025-11-23)

### 已完成
1.  **服务端 (BiliTools)**:
    -   集成 `axum` 搭建 HTTP/WS 服务，端口 `18080`。
    -   实现 `join`, `resolve`, `ws` 等核心接口。
    -   修复接口参数命名风格不一致导致的 400 错误 (camelCase vs snake_case)。
    -   UI 重构：简化同步页面，移除冗余控制，明确“开启服务”与“投送”流程。
    -   放宽权限：允许 Member (如手机端) 在特定场景下触发解析（尽管主要由 Host 操作）。

2.  **客户端 (vo App)**:
    -   UI 重构：实现沉浸式“连接 -> 播放”流程，自动保存连接信息。
    -   引入 `tauri-plugin-http` 解决 WebView 原生 `fetch` 遇到的 CORS 问题。
    -   配置 Android `usesCleartextTraffic` 允许局域网 HTTP 明文传输。
    -   配置 Tauri Capabilities (`http:allow-fetch`) 尝试放行 HTTP 请求 Scope。

### 当前阻塞问题
-   **视频加载与封面同步**:
    -   尽管全链路连通，手机端收到 `room_state`，但视频画面未加载（黑屏/转圈），且封面图仍为默认 Loading 占位符。
    -   原因分析：
        1.  `cover` 字段同步可能存在时序问题或 URL 拼接问题。
        2.  Android 端 WebView 对本地/局域网媒体流的播放兼容性（Codec/HTTPS 混合内容/Range 请求头）。
    -   **待修复**：
        -   检查 `cover` URL 是否为绝对路径或需拼接 base URL。 (已修复：App 端已添加自动拼接逻辑)
        -   排查视频流请求日志，确认 Android 端是否发起了播放请求。


### 下一步计划
1.  **封面与视频修复**:
    -   ✅ 在 App 端对 `cover` 和 `url` 进行 base URL 拼接处理（如果它们是相对路径）。
    -   在服务端添加视频流请求的详细日志 (Range, Headers)。
    -   验证 Android WebView 的混合内容策略（Mixed Content）。
2.  **解决 Scope 问题** (已解决):
    -   创建一个明确的 `src-tauri/capabilities/mobile.json`，并在 `tauri.conf.json` 中通过 `app.security.capabilities` 显式关联。✅ 已完成配置
    -   执行 `tauri android build` 清理缓存，确保配置生效。
2.  **验证全链路**:
    -   连接成功后，测试 WebSocket 心跳与状态同步。
    -   验证 B 站视频解析播放与进度同步。
3.  **优化体验**:
    -   增加连接超时处理与更友好的错误提示（已部分实现）。 
