use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path as AxumPath, Query, State,
    },
    http::response::Builder,
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use axum::extract::rejection::QueryRejection;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use md5;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::{
    fs::File,
    net::TcpListener,
    sync::{mpsc, RwLock},
    task::JoinHandle,
    time as tokio_time,
};
use tokio_util::io::ReaderStream;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use crate::shared::init_client;
use tauri_plugin_http::reqwest;

/// 默认监听端口，桌面端本地服务。
const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:18080";
const ENV_LISTEN_ADDR: &str = "VO_SYNC_ADDR";
const ENV_ALLOW_MEMBER_CONTROL: &str = "VO_ALLOW_MEMBER_CONTROL";

#[derive(Clone)]
struct AppState {
    manager: Arc<Manager>,
    hub: Arc<Hub>,
}

#[derive(Debug, Clone)]
struct SyncConfig {
    listen_addr: String,
    allow_member_control: bool,
}

impl SyncConfig {
    fn from_env() -> Self {
        let listen_addr =
            std::env::var(ENV_LISTEN_ADDR).unwrap_or_else(|_| DEFAULT_LISTEN_ADDR.to_string());
        let allow_member_control = std::env::var(ENV_ALLOW_MEMBER_CONTROL)
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        Self {
            listen_addr,
            allow_member_control,
        }
    }
}

pub async fn init() -> anyhow::Result<()> {
    let cfg = SyncConfig::from_env();
    let manager = Arc::new(Manager::new(None, cfg.allow_member_control));
    manager.spawn_cleanup();
    let hub = Arc::new(Hub::new());
    let (listener, actual_addr) = bind_listener(&cfg.listen_addr).await?;
    let state = AppState {
        manager: manager.clone(),
        hub: hub.clone(),
    };
    tokio::spawn(run_server(state, listener));
    info!(
        "sync service listening on http://{} media_root=unset allow_member_control={}",
        actual_addr, cfg.allow_member_control
    );
    Ok(())
}

async fn run_server(state: AppState, listener: TcpListener) {
    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/room/join", post(join_room))
        .route("/api/media/resolve", post(media_resolve))
        .route("/api/media/root", post(set_media_root).get(get_media_root))
        .route("/media/:token", get(media_stream))
        .route("/ws", get(ws_handler))
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    if let Err(err) = axum::serve(listener, router.into_make_service()).await {
        error!("sync server quit: {err:?}");
    }
}

#[derive(Debug, Deserialize)]
struct JoinRequest {
    room: String,
    password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoinResponse {
    temp_user: String,
    role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaResolveRequest {
    room: String,
    password: String,
    temp_user: String,
    path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaResolveResponse {
    token: String,
    url: String,
    expires_at: i64,
    source_type: String,
    cover: Option<String>,
}

#[derive(Debug)]
struct ResolvedMedia {
    token: String,
    url: String,
    source_type: String,
    cover: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MediaRootRequest {
    path: String,
}

#[derive(Debug, Serialize)]
struct MediaRootResponse {
    media_root: Option<String>,
}

async fn join_room(
    State(state): State<AppState>,
    Json(req): Json<JoinRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (temp_user, is_host) = state.manager.join_room(&req.room, &req.password).await?;
    Ok(Json(JoinResponse {
        temp_user,
        role: if is_host {
            "host".into()
        } else {
            "member".into()
        },
    }))
}

async fn media_resolve(
    State(state): State<AppState>,
    Json(req): Json<MediaResolveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let resolved = state
        .manager
        .resolve_media_path(&req.room, &req.password, &req.temp_user, &req.path)
        .await?;
    let expires_at: i64 = ((OffsetDateTime::now_utc()
        + TimeDuration::seconds(state.manager.token_ttl.as_secs() as i64))
    .unix_timestamp_nanos()
        / 1_000_000)
        .try_into()
        .unwrap_or(i64::MAX);
    
    // 自动创建并广播初始的 room_state
    let initial_state = RoomState {
        url: resolved.url.clone(),
        title: req.path.split('/').last().unwrap_or("视频").to_string(),
        current_time: 0.0,
        duration: 0.0,
        paused: true,
        playback_rate: 1.0,
        source_type: resolved.source_type.clone(),
        updated_at: now_millis(),
        cover: resolved.cover.clone(),
    };
    
    // 更新房间状态
    let updated_state = state
        .manager
        .update_state(&req.room, &req.temp_user, initial_state, true)
        .await?;
    
    // 广播给所有客户端
    state.hub.broadcast_state(&req.room, &updated_state).await;
    
    Ok(Json(MediaResolveResponse {
        url: resolved.url,
        token: resolved.token,
        expires_at,
        source_type: resolved.source_type,
        cover: resolved.cover,
    }))
}

async fn set_media_root(
    State(state): State<AppState>,
    Json(req): Json<MediaRootRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let path = state.manager.set_media_root(&req.path).await?;
    Ok(Json(MediaRootResponse {
        media_root: path.to_str().map(|s| s.to_string()),
    }))
}

async fn get_media_root(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let path = state.manager.media_root.read().await.clone();
    Ok(Json(MediaRootResponse {
        media_root: path
            .as_ref()
            .and_then(|p| p.to_str().map(|s| s.to_string())),
    }))
}

async fn media_stream(
    State(state): State<AppState>,
    AxumPath(token): AxumPath<String>,
    req: Request<Body>,
) -> Result<Response, ApiError> {
    if let Ok(target) = state.manager.open_remote(&token).await {
        match target.strategy {
            RemoteStrategy::Redirect => {
                return Ok(Response::builder()
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .header(
                        axum::http::header::LOCATION,
                        HeaderValue::from_str(&target.url).unwrap_or(HeaderValue::from_static("")),
                    )
                    .body(Body::empty())
                    .unwrap());
            }
            RemoteStrategy::ProxyWithHeaders => {
                let client = init_client()
                    .await
                    .map_err(|e| ApiError::bad_request(format!("client init failed: {e}")))?;
                let mut builder = client.get(&target.url);
                if let Some(range) = req.headers().get(axum::http::header::RANGE) {
                    builder = builder.header(axum::http::header::RANGE, range.clone());
                }
                builder = builder.header(axum::http::header::REFERER, "https://www.bilibili.com/");
                let upstream = builder
                    .send()
                    .await
                    .map_err(|e| ApiError::not_found(format!("upstream error: {e}")))?;
                let status =
                    StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
                let mut resp_builder = Response::builder().status(status);
                let headers = upstream.headers();
                copy_header(headers, axum::http::header::CONTENT_TYPE, &mut resp_builder);
                copy_header(
                    headers,
                    axum::http::header::CONTENT_LENGTH,
                    &mut resp_builder,
                );
                copy_header(
                    headers,
                    axum::http::header::ACCEPT_RANGES,
                    &mut resp_builder,
                );
                copy_header(
                    headers,
                    axum::http::header::CONTENT_RANGE,
                    &mut resp_builder,
                );
                let stream = upstream.bytes_stream();
                let body = Body::from_stream(stream);
                return resp_builder
                    .body(body)
                    .map_err(|e| ApiError::bad_request(format!("build body failed: {e}")));
            }
        }
    }

    let path = state.manager.open_media(&token).await?;
    let file = File::open(&path)
        .await
        .map_err(|_| ApiError::not_found("media not found"))?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(body)
        .unwrap())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsQuery {
    room: String,
    password: String,
    temp_user: String,
}

async fn ws_handler(
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
    State(state): State<AppState>,
    query: Result<Query<WsQuery>, QueryRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let ws = match ws {
        Ok(v) => v,
        Err(e) => {
            warn!("ws upgrade rejection: {}", e);
            return Ok((e.status(), e.to_string()).into_response());
        }
    };
    let query = match query {
        Ok(v) => v.0,
        Err(e) => {
            warn!("ws query rejection: {}", e);
            return Ok((e.status(), e.to_string()).into_response());
        }
    };
    let is_host = match state
        .manager
        .authorize(&query.room, &query.password, &query.temp_user)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!(
                "ws authorize failed room={} user={} err={}",
                query.room, query.temp_user, e
            );
            // return a plain 403 for clarity
            return Ok((StatusCode::FORBIDDEN, e.to_string()).into_response());
        }
    };
    let ctx = WsContext {
        room: query.room.clone(),
        temp_user: query.temp_user.clone(),
        is_host,
    };
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, ctx)))
}

#[derive(Clone)]
struct WsContext {
    room: String,
    temp_user: String,
    is_host: bool,
}

async fn handle_socket(socket: WebSocket, state: AppState, ctx: WsContext) {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let client_id = Uuid::new_v4().to_string();
    state
        .hub
        .register(&ctx.room, &client_id, out_tx.clone())
        .await;

    if let Some(current) = state.manager.current_state(&ctx.room).await {
        if let Ok(payload) = serde_json::to_string(&WsOutgoing {
            r#type: "room_state".into(),
            state: Some(current),
            error: None,
        }) {
            let _ = out_tx.send(Message::Text(payload));
        }
    } else {
        // DEBUG: 即使没有状态，也发送一条消息证明连接成功
        let payload = serde_json::json!({
            "type": "debug_info",
            "message": "Connected! Waiting for host push..."
        }).to_string();
        let _ = out_tx.send(Message::Text(payload));
    }

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut send_task: JoinHandle<()> = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    let manager = state.manager.clone();
    let hub = state.hub.clone();
    let recv_ctx = ctx.clone();
    let recv_client_id = client_id.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            if let Err(err) = handle_ws_message(msg, &manager, &hub, &recv_ctx).await {
                warn!("ws message error: {err:?}");
                let _ = hub
                    .send_to(
                        &recv_ctx.room,
                        &recv_client_id,
                        WsOutgoing::error(err.message.clone()),
                    )
                    .await;
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    }

    state.hub.unregister(&ctx.room, &client_id).await;
}

async fn handle_ws_message(
    msg: Message,
    manager: &Arc<Manager>,
    hub: &Arc<Hub>,
    ctx: &WsContext,
) -> Result<(), ApiError> {
    match msg {
        Message::Text(text) => {
            let incoming: WsIncoming = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    warn!("ws deserialize error: {} | input: {}", e, text);
                    return Err(ApiError::bad_request("invalid message format"));
                }
            };
            match incoming.r#type.as_str() {
                "host_update" => {
                    let state = incoming
                        .state
                        .ok_or_else(|| ApiError::bad_request("state required"))?;
                    let updated = manager
                        .update_state(&ctx.room, &ctx.temp_user, state, ctx.is_host)
                        .await?;
                    hub.broadcast_state(&ctx.room, &updated).await;
                }
                "member_ping" => {
                    manager.touch_member(&ctx.room, &ctx.temp_user).await;
                }
                _ => return Err(ApiError::bad_request("unknown message type")),
            }
        }
        Message::Close(_) => {}
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct WsIncoming {
    #[serde(rename = "type")]
    r#type: String,
    state: Option<RoomState>,
}

#[derive(Debug, Serialize)]
struct WsOutgoing {
    #[serde(rename = "type")]
    r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<RoomState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl WsOutgoing {
    fn error(msg: impl Into<String>) -> Self {
        Self {
            r#type: "error".into(),
            state: None,
            error: Some(msg.into()),
        }
    }
}

/// 简化的错误响应封装，返回统一 JSON。
#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }

    fn forbidden(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: msg.into(),
        }
    }

    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomState {
    pub url: String,
    pub title: String,
    pub current_time: f64,
    pub duration: f64,
    pub paused: bool,
    pub playback_rate: f64,
    pub source_type: String,
    pub updated_at: i64,
    #[serde(default)]
    pub cover: Option<String>,
}

#[derive(Debug, Clone)]
struct Room {
    password: String,
    host_id: Option<String>,
    state: Option<RoomState>,
    members: HashMap<String, Instant>,
    last_update: Option<Instant>,
}

#[derive(Debug, Clone)]
enum MediaTarget {
    Local(PathBuf),
    Remote(RemoteTarget),
}

#[derive(Debug, Clone)]
struct MediaToken {
    target: MediaTarget,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
enum RemoteStrategy {
    Redirect,
    ProxyWithHeaders,
}

#[derive(Debug, Clone)]
struct RemoteTarget {
    url: String,
    strategy: RemoteStrategy,
}

#[derive(Debug)]
struct Manager {
    rooms: RwLock<HashMap<String, Room>>,
    media_tokens: RwLock<HashMap<String, MediaToken>>,
    media_root: RwLock<Option<PathBuf>>,
    room_ttl: Duration,
    token_ttl: Duration,
    allow_member_control: bool,
}

impl Manager {
    fn new(media_root: Option<PathBuf>, allow_member_control: bool) -> Self {
        Self {
            rooms: RwLock::new(HashMap::new()),
            media_tokens: RwLock::new(HashMap::new()),
            media_root: RwLock::new(media_root.map(clean_path)),
            room_ttl: Duration::from_secs(30 * 60),
            token_ttl: Duration::from_secs(60 * 60),
            allow_member_control,
        }
    }

    fn spawn_cleanup(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticker = tokio_time::interval(Duration::from_secs(60));
            loop {
                ticker.tick().await;
                if let Some(manager) = weak.upgrade() {
                    manager.cleanup().await;
                } else {
                    break;
                }
            }
        });
    }

    async fn join_room(&self, name: &str, password: &str) -> Result<(String, bool), ApiError> {
        let name = name.trim();
        let password = password.trim();
        if name.is_empty() || password.is_empty() {
            return Err(ApiError::bad_request("room name and password required"));
        }
        let temp_user = Uuid::new_v4().to_string();
        let mut rooms = self.rooms.write().await;
        let room = rooms.entry(name.to_string()).or_insert_with(|| Room {
            password: password.to_string(),
            host_id: None,
            state: None,
            members: HashMap::new(),
            last_update: None,
        });
        if room.password != password {
            return Err(ApiError::bad_request("room password mismatch"));
        }
        let mut is_host = false;
        if room.host_id.is_none() {
            room.host_id = Some(temp_user.clone());
            is_host = true;
        }
        room.members.insert(temp_user.clone(), Instant::now());
        Ok((temp_user, is_host))
    }

    async fn authorize(
        &self,
        room_name: &str,
        password: &str,
        temp_user: &str,
    ) -> Result<bool, ApiError> {
        let rooms = self.rooms.read().await;
        let room = rooms
            .get(room_name)
            .ok_or_else(|| ApiError::forbidden("room not found"))?;
        if room.password != password {
            return Err(ApiError::forbidden("room password mismatch"));
        }
        if !room.members.contains_key(temp_user) {
            return Err(ApiError::forbidden("user not in room"));
        }
        Ok(room
            .host_id
            .as_ref()
            .map(|id| id == temp_user)
            .unwrap_or(false))
    }

    async fn touch_member(&self, room_name: &str, temp_user: &str) {
        if let Some(room) = self.rooms.write().await.get_mut(room_name) {
            room.members.insert(temp_user.to_string(), Instant::now());
        }
    }

    async fn update_state(
        &self,
        room_name: &str,
        _temp_user: &str,
        mut state: RoomState,
        is_host: bool,
    ) -> Result<RoomState, ApiError> {
        let mut rooms = self.rooms.write().await;
        let room = rooms
            .get_mut(room_name)
            .ok_or_else(|| ApiError::bad_request("room not found"))?;
        if is_host {
            state.updated_at = now_millis();
            room.state = Some(state.clone());
            room.last_update = Some(Instant::now());
            return Ok(state);
        }

        if !self.allow_member_control {
            return Err(ApiError::forbidden("operation allowed for host only"));
        }
        let existing = room
            .state
            .clone()
            .ok_or_else(|| ApiError::bad_request("host has not published state"))?;
        // 成员只能调整播放进度/暂停/倍速，不能切换源。
        let merged = RoomState {
            url: existing.url,
            title: existing.title,
            duration: existing.duration,
            source_type: existing.source_type,
            current_time: state.current_time,
            paused: state.paused,
            playback_rate: state.playback_rate,
            updated_at: now_millis(),
            cover: existing.cover,
        };
        room.state = Some(merged.clone());
        room.last_update = Some(Instant::now());
        Ok(merged)
    }

    async fn current_state(&self, room_name: &str) -> Option<RoomState> {
        let rooms = self.rooms.read().await;
        rooms.get(room_name).and_then(|room| room.state.clone())
    }

    async fn resolve_media_path(
        &self,
        room_name: &str,
        password: &str,
        _temp_user: &str,
        path: &str,
    ) -> Result<ResolvedMedia, ApiError> {
        let rooms = self.rooms.read().await;
        let room = rooms
            .get(room_name)
            .ok_or_else(|| ApiError::bad_request("room not found"))?;
        if room.password != password {
            return Err(ApiError::forbidden("room password mismatch"));
        }
        if room.host_id.as_deref() != Some(_temp_user) && !self.allow_member_control {
            return Err(ApiError::forbidden("operation allowed for host only"));
        }
        drop(rooms);

        if let Some(_) = is_bilibili_source(path) {
            let resolved = self.resolve_bilibili(path).await?;
            return Ok(resolved);
        }

        if path.starts_with("http://") || path.starts_with("https://") {
            let token = Uuid::new_v4().to_string();
            self.media_tokens.write().await.insert(
                token.clone(),
                MediaToken {
                    target: MediaTarget::Remote(RemoteTarget {
                        url: path.to_string(),
                        strategy: RemoteStrategy::Redirect,
                    }),
                    expires_at: Instant::now() + self.token_ttl,
                },
            );
            return Ok(ResolvedMedia {
                url: format!("/media/{token}"),
                token,
                source_type: "remote".into(),
                cover: None,
            });
        }

        let root = self
            .media_root
            .read()
            .await
            .clone()
            .ok_or_else(|| ApiError::bad_request("media root not configured"))?;
        let path = PathBuf::from(path);
        let clean = clean_path(path);
        if !is_under_root(&clean, &root) {
            return Err(ApiError::forbidden("media path forbidden"));
        }
        let meta = std::fs::metadata(&clean).map_err(|_| ApiError::bad_request("invalid path"))?;
        if meta.is_dir() {
            return Err(ApiError::bad_request("path is directory"));
        }

        let token = Uuid::new_v4().to_string();
        self.media_tokens.write().await.insert(
            token.clone(),
            MediaToken {
                target: MediaTarget::Local(clean),
                expires_at: Instant::now() + self.token_ttl,
            },
        );
        Ok(ResolvedMedia {
            url: format!("/media/{token}"),
            token,
            source_type: "file".into(),
            cover: None,
        })
    }

    async fn open_media(&self, token: &str) -> Result<PathBuf, ApiError> {
        let tokens = self.media_tokens.read().await;
        let entry = tokens
            .get(token)
            .ok_or_else(|| ApiError::not_found("token not found"))?;
        if Instant::now() > entry.expires_at {
            return Err(ApiError::not_found("token expired"));
        }
        match &entry.target {
            MediaTarget::Local(p) => Ok(p.clone()),
            MediaTarget::Remote(_) => Err(ApiError::bad_request("remote requires redirect")),
        }
    }

    async fn open_remote(&self, token: &str) -> Result<RemoteTarget, ApiError> {
        let tokens = self.media_tokens.read().await;
        let entry = tokens
            .get(token)
            .ok_or_else(|| ApiError::not_found("token not found"))?;
        if Instant::now() > entry.expires_at {
            return Err(ApiError::not_found("token expired"));
        }
        match &entry.target {
            MediaTarget::Remote(target) => Ok(target.clone()),
            MediaTarget::Local(_) => Err(ApiError::bad_request("not a remote token")),
        }
    }

    async fn set_media_root(&self, path: &str) -> Result<PathBuf, ApiError> {
        let candidate = clean_path(path);
        let meta = std::fs::metadata(&candidate)
            .map_err(|_| ApiError::bad_request("media root not found"))?;
        if !meta.is_dir() {
            return Err(ApiError::bad_request("media root must be directory"));
        }
        *self.media_root.write().await = Some(candidate.clone());
        Ok(candidate)
    }

    async fn resolve_bilibili(&self, input: &str) -> Result<ResolvedMedia, ApiError> {
        let bvid =
            extract_bvid(input).ok_or_else(|| ApiError::bad_request("invalid bilibili id"))?;
        let client = init_client()
            .await
            .map_err(|e| ApiError::bad_request(format!("client init failed: {e}")))?;

        let view: ViewResp = client
            .get("https://api.bilibili.com/x/web-interface/view")
            .query(&[("bvid", &bvid)])
            .send()
            .await
            .map_err(|e| ApiError::bad_request(format!("view request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ApiError::bad_request(format!("view parse failed: {e}")))?;

        let cid = view.data.cid;
        let mut params = BTreeMap::new();
        params.insert("bvid".into(), bvid.clone());
        params.insert("cid".into(), cid.to_string());
        params.insert("qn".into(), "112".into()); // 1080P+ 高码率
        params.insert("fnval".into(), "1".into()); // MP4 格式（包含音频），fnval=16 是 DASH（音视频分离）
        params.insert("fourk".into(), "1".into());

        let query = wbi_sign(&client, params).await?;
        let play_url = format!("https://api.bilibili.com/x/player/wbi/playurl?{query}");
        let play_resp: PlayUrlResp = client
            .get(play_url)
            .send()
            .await
            .map_err(|e| ApiError::bad_request(format!("playurl request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ApiError::bad_request(format!("playurl parse failed: {e}")))?;
        if play_resp.code != 0 {
            return Err(ApiError::bad_request(format!(
                "playurl error: {}",
                play_resp.message
            )));
        }
        // 使用传统 durl 格式（MP4，包含音频）
        let media_url = if let Some(d) = play_resp.data.durl.first() {
            d.url.clone()
        } else if let Some(dash) = &play_resp.data.dash {
            // DASH 格式音视频分离，需要客户端支持 MSE，这里暂不支持
            return Err(ApiError::bad_request("DASH format not supported (audio/video separated)"));
        } else {
            return Err(ApiError::bad_request("no playable stream"));
        };

        let token = Uuid::new_v4().to_string();
        self.media_tokens.write().await.insert(
            token.clone(),
            MediaToken {
                target: MediaTarget::Remote(RemoteTarget {
                    url: media_url,
                    strategy: RemoteStrategy::ProxyWithHeaders,
                }),
                expires_at: Instant::now() + self.token_ttl,
            },
        );
        Ok(ResolvedMedia {
            url: format!("/media/{token}"),
            token,
            source_type: "bili".into(),
            cover: view.data.pic,
        })
    }

    async fn cleanup(&self) {
        let mut rooms = self.rooms.write().await;
        let mut tokens = self.media_tokens.write().await;
        let now = Instant::now();
        rooms.retain(|_, room| {
            let mut last_seen = room.last_update.unwrap_or(now);
            for seen in room.members.values() {
                if *seen > last_seen {
                    last_seen = *seen;
                }
            }
            now.duration_since(last_seen) <= self.room_ttl
        });
        tokens.retain(|_, token| now <= token.expires_at);
    }
}

#[derive(Clone)]
struct Hub {
    clients: Arc<RwLock<HashMap<String, HashMap<String, mpsc::UnboundedSender<Message>>>>>,
}

impl Hub {
    fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn register(&self, room: &str, client_id: &str, tx: mpsc::UnboundedSender<Message>) {
        let mut clients = self.clients.write().await;
        let room_clients = clients.entry(room.to_string()).or_default();
        room_clients.insert(client_id.to_string(), tx);
    }

    async fn unregister(&self, room: &str, client_id: &str) {
        let mut clients = self.clients.write().await;
        if let Some(room_clients) = clients.get_mut(room) {
            room_clients.remove(client_id);
            if room_clients.is_empty() {
                clients.remove(room);
            }
        }
    }

    async fn broadcast_state(&self, room: &str, state: &RoomState) {
        let payload = Message::Text(
            serde_json::to_string(&WsOutgoing {
                r#type: "room_state".into(),
                state: Some(state.clone()),
                error: None,
            })
            .unwrap(),
        );
        let mut clients = self.clients.write().await;
        if let Some(room_clients) = clients.get_mut(room) {
            room_clients.retain(|_, tx| tx.send(payload.clone()).is_ok());
        }
    }

    async fn send_to(&self, room: &str, client_id: &str, msg: WsOutgoing) -> Result<(), ApiError> {
        let mut clients = self.clients.write().await;
        if let Some(room_clients) = clients.get_mut(room) {
            if let Some(tx) = room_clients.get(client_id) {
                let payload =
                    Message::Text(serde_json::to_string(&msg).unwrap_or_else(|_| "{}".into()));
                tx.send(payload)
                    .map_err(|_| ApiError::bad_request("send error"))?;
                return Ok(());
            }
        }
        Err(ApiError::not_found("client not found"))
    }
}

async fn bind_listener(addr: &str) -> anyhow::Result<(TcpListener, SocketAddr)> {
    let mut candidates = Vec::new();
    candidates.push(addr.to_string());
    if addr == DEFAULT_LISTEN_ADDR {
        for port in 18080..18090 {
            candidates.push(format!("127.0.0.1:{port}"));
        }
        candidates.push("127.0.0.1:0".to_string());
    }
    for candidate in candidates {
        match TcpListener::bind(&candidate).await {
            Ok(listener) => {
                let local = listener.local_addr()?;
                if candidate != addr {
                    warn!("sync server fallback to {}", local);
                }
                return Ok((listener, local));
            }
            Err(err) => {
                warn!("bind {} failed: {err}", candidate);
            }
        }
    }
    Err(anyhow::anyhow!(
        "failed to bind sync server; consider setting {}",
        ENV_LISTEN_ADDR
    ))
}

fn now_millis() -> i64 {
    let now = OffsetDateTime::now_utc();
    (now.unix_timestamp_nanos() / 1_000_000)
        .try_into()
        .unwrap_or(i64::MAX)
}

fn clean_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let p = path.as_ref();
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn is_under_root(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

fn is_bilibili_source(input: &str) -> Option<&str> {
    if input.starts_with("BV")
        || input.starts_with("bv")
        || input.starts_with("ep")
        || input.starts_with("EP")
    {
        return Some(input);
    }
    if input.starts_with("http://") || input.starts_with("https://") {
        if input.contains("bilibili.com") || input.contains("bilivideo.com") {
            return Some(input);
        }
    }
    None
}

fn copy_header(headers: &HeaderMap, key: axum::http::header::HeaderName, builder: &mut Builder) {
    if let Some(val) = headers.get(&key) {
        if let Some(map) = builder.headers_mut() {
            map.insert(key.clone(), val.clone());
        }
    }
}

async fn log_requests(req: Request<Body>, next: Next) -> impl IntoResponse {
    info!("sync request {} {}", req.method(), req.uri());
    let res = next.run(req).await;
    res
}

const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

#[derive(Debug, Deserialize)]
struct NavResp {
    data: NavData,
}

#[derive(Debug, Deserialize)]
struct NavData {
    wbi_img: WbiImg,
}

#[derive(Debug, Deserialize)]
struct WbiImg {
    img_url: String,
    sub_url: String,
}

#[derive(Debug, Deserialize)]
struct ViewResp {
    data: ViewData,
}

#[derive(Debug, Deserialize)]
struct ViewData {
    bvid: String,
    cid: i64,
    title: String,
    #[serde(default)]
    pic: Option<String>, // Bilibili cover
    #[serde(default)]
    duration: i64,
}

#[derive(Debug, Deserialize)]
struct PlayUrlResp {
    code: i32,
    message: String,
    data: PlayUrlData,
}

#[derive(Debug, Deserialize)]
struct PlayUrlData {
    #[serde(default)]
    durl: Vec<Durl>,
    #[serde(default)]
    dash: Option<Dash>,
}

#[derive(Debug, Deserialize)]
struct Durl {
    url: String,
}

#[derive(Debug, Deserialize)]
struct Dash {
    #[serde(default)]
    video: Vec<DashStream>,
}

#[derive(Debug, Deserialize)]
struct DashStream {
    #[serde(rename = "baseUrl")]
    base_url: String,
}

fn extract_bvid(input: &str) -> Option<String> {
    if let Some(idx) = input.find("BV") {
        let slice = &input[idx..];
        let mut bvid = String::new();
        for c in slice.chars() {
            if c.is_alphanumeric() {
                bvid.push(c);
                if bvid.len() >= 12 {
                    break;
                }
            } else if !bvid.is_empty() {
                break;
            }
        }
        if bvid.starts_with("BV") && bvid.len() >= 10 {
            return Some(bvid);
        }
    }
    if input.starts_with("BV") && input.len() >= 10 {
        return Some(input.to_string());
    }
    None
}

async fn wbi_sign(
    client: &reqwest::Client,
    mut params: BTreeMap<String, String>,
) -> Result<String, ApiError> {
    let nav: NavResp = client
        .get("https://api.bilibili.com/x/web-interface/nav")
        .send()
        .await
        .map_err(|e| ApiError::bad_request(format!("nav request failed: {e}")))?
        .json()
        .await
        .map_err(|e| ApiError::bad_request(format!("nav parse failed: {e}")))?;
    let img_key = nav
        .data
        .wbi_img
        .img_url
        .rsplit('/')
        .next()
        .and_then(|s| s.split('.').next())
        .unwrap_or("");
    let sub_key = nav
        .data
        .wbi_img
        .sub_url
        .rsplit('/')
        .next()
        .and_then(|s| s.split('.').next())
        .unwrap_or("");
    let mixin_source = format!("{img_key}{sub_key}");
    let mixin_key: String = MIXIN_KEY_ENC_TAB
        .iter()
        .filter_map(|idx| mixin_source.chars().nth(*idx))
        .take(32)
        .collect();
    let curr_time = OffsetDateTime::now_utc().unix_timestamp();
    params.insert("wts".into(), curr_time.to_string());

    let chr_filter = |s: &str| s.replace(&['\'', '!', '(', ')', '*'][..], "");
    let query = params
        .iter()
        .map(|(k, v)| (k.as_str(), chr_filter(v)))
        .collect::<Vec<_>>();
    let mut query_sorted = query;
    query_sorted.sort_by(|a, b| a.0.cmp(b.0));
    let encoded = query_sorted
        .into_iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, NON_ALPHANUMERIC),
                utf8_percent_encode(&v, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let sign = format!("{:x}", md5::compute(format!("{encoded}{mixin_key}")));
    Ok(format!("{encoded}&w_rid={sign}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File as StdFile, io::Write};

    #[tokio::test]
    async fn join_and_authorize_flow() {
        let manager = Manager::new(None, true);
        let (host, is_host) = manager.join_room("room", "pwd").await.unwrap();
        assert!(is_host);
        let (member, member_host) = manager.join_room("room", "pwd").await.unwrap();
        assert!(!member_host);
        assert!(manager.authorize("room", "pwd", &host).await.unwrap());
        assert!(!manager.authorize("room", "pwd", &member).await.unwrap());
    }

    #[tokio::test]
    async fn member_control_merges_playback_only() {
        let manager = Manager::new(None, true);
        let (host, _) = manager.join_room("room", "pwd").await.unwrap();
        let (member, _) = manager.join_room("room", "pwd").await.unwrap();
        let host_state = RoomState {
            url: "file:///movie.mp4".into(),
            title: "Movie".into(),
            current_time: 0.0,
            duration: 120.0,
            paused: false,
            playback_rate: 1.0,
            source_type: "file".into(),
            updated_at: 0,
            cover: None,
        };
        manager
            .update_state("room", &host, host_state, true)
            .await
            .unwrap();

        let member_update = RoomState {
            url: "hijack".into(),
            title: "hijack".into(),
            current_time: 30.0,
            duration: 999.0,
            paused: true,
            playback_rate: 1.5,
            source_type: "other".into(),
            updated_at: 0,
            cover: None,
        };
        let merged = manager
            .update_state("room", &member, member_update, false)
            .await
            .unwrap();
        assert_eq!(merged.url, "file:///movie.mp4");
        assert_eq!(merged.title, "Movie");
        assert_eq!(merged.duration, 120.0);
        assert_eq!(merged.source_type, "file");
        assert_eq!(merged.current_time, 30.0);
        assert_eq!(merged.paused, true);
        assert_eq!(merged.playback_rate, 1.5);
    }

    #[tokio::test]
    async fn resolve_local_media_and_block_online() {
        let root = std::env::temp_dir().join("vo_sync_test");
        std::fs::create_dir_all(&root).unwrap();
        let file_path = root.join("sample.mp4");
        let mut file = StdFile::create(&file_path).unwrap();
        writeln!(file, "dummy").unwrap();

        let manager = Manager::new(Some(root.clone()), true);
        let (host, _) = manager.join_room("room", "pwd").await.unwrap();
        let res = manager
            .resolve_media_path("room", "pwd", &host, file_path.to_str().unwrap())
            .await
            .expect("should generate token");
        assert_eq!(res.source_type, "file");
        assert!(res.url.contains("/media/"));
        assert!(!res.token.is_empty());

        let err = manager
            .resolve_media_path("room", "pwd", &host, "https://bilibili.com/video/BVxxx")
            .await
            .unwrap_err();
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let remote = manager
            .resolve_media_path("room", "pwd", &host, "https://example.com/video.mp4")
            .await
            .expect("remote should be tokenized");
        assert_eq!(remote.source_type, "remote");
        assert!(remote.url.contains("/media/"));
    }

    #[tokio::test]
    async fn set_media_root_and_resolve_local() {
        let root = std::env::temp_dir().join("vo_sync_root_set");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Manager::new(None, true);
        manager
            .set_media_root(root.to_str().unwrap())
            .await
            .expect("set media root");
        let file_path = root.join("movie.mp4");
        let mut file = StdFile::create(&file_path).unwrap();
        writeln!(file, "dummy").unwrap();
        let (host, _) = manager.join_room("r", "p").await.unwrap();
        let res = manager
            .resolve_media_path("r", "p", &host, file_path.to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(res.source_type, "file");
    }
}
