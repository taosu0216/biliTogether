import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import "./App.css";
import { SyncSession, useSyncClient } from "./lib/syncClient";
import { fetch } from '@tauri-apps/plugin-http';
import { getCurrentWindow } from '@tauri-apps/api/window';

function sanitizeBaseUrl(input: string) {
  if (!input) return "";
  try {
    const parsed = new URL(input);
    const cleanPath = parsed.pathname.replace(/\/$/, "");
    return `${parsed.origin}${cleanPath}`;
  } catch {
    return input.replace(/\/$/, "");
  }
}

function App() {
  const [serverUrl, setServerUrl] = useState(localStorage.getItem("vo_server_url") || "");
  const [room, setRoom] = useState(localStorage.getItem("vo_room") || "default");
  const [password, setPassword] = useState(localStorage.getItem("vo_password") || "123");
  
  const [status, setStatus] = useState<string>();
  const [error, setError] = useState<string>();
  const [session, setSession] = useState<SyncSession | null>(null);

  const baseUrl = useMemo(() => sanitizeBaseUrl(serverUrl.trim()), [serverUrl]);
  const syncClient = useSyncClient(session);
  const videoRef = useRef<HTMLVideoElement>(null);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [showControls, setShowControls] = useState(true);
  const [fitMode, setFitMode] = useState<'contain' | 'cover'>('contain');
  const controlsTimeoutRef = useRef<ReturnType<typeof window.setTimeout>>(undefined);

  // 保存配置
  useEffect(() => {
    localStorage.setItem("vo_server_url", serverUrl);
    localStorage.setItem("vo_room", room);
    localStorage.setItem("vo_password", password);
  }, [serverUrl, room, password]);

  const isConnected = !!session;
  const hasMedia = !!syncClient.state?.url;

  // Helper to resolve full URL
  const resolveUrl = (url?: string) => {
    if (!url) return undefined;
    if (url.startsWith("http://") || url.startsWith("https://")) {
      return url;
    }
    if (url.startsWith("/") && session?.serverUrl) {
      const base = session.serverUrl.replace(/\/+$/, "");
      return `${base}${url}`;
    }
    return url;
  };

  const videoUrl = resolveUrl(syncClient.state?.url);
  const coverUrl = resolveUrl(syncClient.state?.cover);

  const handleJoin = async (event: FormEvent) => {
    event.preventDefault();
    console.log("Starting join process...");
    setStatus("正在连接...");
    setError(undefined);

    if (!baseUrl || !room || !password) {
      setError("请填写完整信息");
      return;
    }

    try {
      console.log(`Connecting to ${baseUrl}/api/room/join`);
      const resp = await fetch(`${baseUrl}/api/room/join`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ room: room.trim(), password: password.trim() }),
        connectTimeout: 5000, // 5s timeout for connection (Tauri specific option if supported, otherwise ignored)
      });
      
      console.log("Response status:", resp.status);

      if (!resp.ok) {
        const text = await resp.text();
        throw new Error(`连接失败: ${resp.status} ${text}`);
      }

      const data = (await resp.json()) as { tempUser: string; role: SyncSession["role"] };
      console.log("Join success:", data);
      
      setSession({
        serverUrl: baseUrl,
        room: room.trim(),
        password: password.trim(),
        tempUser: data.tempUser,
        role: data.role ?? "member",
      });
      
      setStatus(undefined);
    } catch (err: any) {
      console.error("Join error:", err);
      const msg = err.message || String(err);
      setError(msg);
      // 在移动端弹出 alert 以便调试
      alert(`连接出错: ${msg}\n请检查 IP 是否正确以及防火墙设置。`);
      setStatus(undefined);
    }
  };

  // 控制栏自动隐藏
  useEffect(() => {
    if (showControls && hasMedia) {
      if (controlsTimeoutRef.current) window.clearTimeout(controlsTimeoutRef.current);
      controlsTimeoutRef.current = window.setTimeout(() => {
        // 只有在播放中时才自动隐藏
        if (!videoRef.current?.paused) {
          setShowControls(false);
        }
      }, 3000);
    }
    return () => {
      if (controlsTimeoutRef.current) window.clearTimeout(controlsTimeoutRef.current);
    };
  }, [showControls, hasMedia]);

  // 1. 同步服务端状态到视频元素（Member 接收 Host 的状态）
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !syncClient.state || !videoUrl) return;

    const state = syncClient.state;
    const timeDiff = Math.abs(video.currentTime - state.currentTime);
    
    // 只在时间差距较大时才同步，避免频繁跳转
    if (timeDiff > 2) {
      console.log(`⏱️ Syncing time: ${video.currentTime.toFixed(1)}s → ${state.currentTime.toFixed(1)}s`);
      video.currentTime = state.currentTime;
    }

    if (state.paused && !video.paused) {
      console.log("⏸️ Pausing video");
      video.pause();
    } else if (!state.paused && video.paused) {
      console.log("▶️ Playing video");
      video.play().catch(err => console.warn("Auto play blocked:", err));
    }

    if (Math.abs(video.playbackRate - state.playbackRate) > 0.01) {
      video.playbackRate = state.playbackRate;
    }
  }, [syncClient.state, videoUrl]);


  // 2. 监听本地视频事件，实时发送状态到服务端
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !session || !videoUrl) return;

    let isSyncing = false; // 防止同步时触发事件导致循环

    const sendUpdate = () => {
      if (isSyncing) return;
      
      const state = {
        url: videoUrl,
        title: syncClient.state?.title || "视频",
        currentTime: video.currentTime,
        duration: video.duration || 0,
        paused: video.paused,
        playbackRate: video.playbackRate,
        sourceType: syncClient.state?.sourceType || "unknown",
        updatedAt: Date.now(),
        cover: coverUrl,
      };
      console.log("📤 Sending state:", { 
        time: state.currentTime.toFixed(1), 
        paused: state.paused 
      });
      syncClient.sendHostUpdate(state);
    };

    // 监听用户操作事件
    video.addEventListener('play', sendUpdate);
    video.addEventListener('pause', sendUpdate);
    video.addEventListener('seeked', sendUpdate);
    video.addEventListener('ratechange', sendUpdate);
    
    // 定期发送进度（播放时每0.5秒同步一次）
    const interval = setInterval(() => {
      if (!video.paused) {
        sendUpdate();
      }
    }, 500);

    return () => {
      video.removeEventListener('play', sendUpdate);
      video.removeEventListener('pause', sendUpdate);
      video.removeEventListener('seeked', sendUpdate);
      video.removeEventListener('ratechange', sendUpdate);
      clearInterval(interval);
    };
  }, [session, videoUrl, coverUrl, syncClient]);

  // 3. 接收并应用服务端广播的状态
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !session || !syncClient.state) return;

    const state = syncClient.state;
    const SYNC_THRESHOLD = 1.0; // 超过1秒差异才同步进度

    console.log('📥 Received state:', {
      remoteTime: state.currentTime.toFixed(1),
      remotePaused: state.paused,
      localTime: video.currentTime.toFixed(1),
      localPaused: video.paused,
    });

    // 等待视频加载
    if (video.readyState < 2) {
      const onLoadedData = () => {
        console.log('📹 Video loaded');
        video.currentTime = state.currentTime;
        if (!state.paused) {
          video.play().catch(e => console.error('Play failed:', e));
        }
        video.removeEventListener('loadeddata', onLoadedData);
      };
      video.addEventListener('loadeddata', onLoadedData);
      return () => video.removeEventListener('loadeddata', onLoadedData);
    }

    // 同步播放速率
    if (Math.abs(video.playbackRate - state.playbackRate) > 0.01) {
      video.playbackRate = state.playbackRate;
    }

    // 同步播放/暂停状态
    if (state.paused && !video.paused) {
      console.log('⏸️ Pausing (remote)');
      video.pause();
    } else if (!state.paused && video.paused) {
      console.log('▶️ Playing (remote)');
      video.play().catch(e => console.error('Play failed:', e));
    }

    // 同步进度
    const timeDiff = Math.abs(video.currentTime - state.currentTime);
    if (timeDiff > SYNC_THRESHOLD) {
      console.log(`⏩ Seeking to ${state.currentTime.toFixed(1)}s (diff: ${timeDiff.toFixed(1)}s)`);
      video.currentTime = state.currentTime;
    }
  }, [syncClient.state, session]);

  const toggleFullscreen = async () => {
    try {
      const win = getCurrentWindow();
      const isFull = await win.isFullscreen();
      if (isFull) {
        await win.setFullscreen(false);
        setIsFullscreen(false);
      } else {
        await win.setFullscreen(true);
        setIsFullscreen(true);
      }
    } catch (e) {
      console.error('Toggle fullscreen failed:', e);
      // Fallback to web fullscreen API
      if (!document.fullscreenElement) {
         document.documentElement.requestFullscreen().catch(console.error);
         setIsFullscreen(true);
      } else {
        document.exitFullscreen().catch(console.error);
        setIsFullscreen(false);
      }
    }
  };

  const togglePlay = () => {
    if (!videoRef.current) return;
    if (videoRef.current.paused) {
      videoRef.current.play();
    } else {
      videoRef.current.pause();
    }
  };

  const toggleFitMode = () => {
    setFitMode(prev => prev === 'contain' ? 'cover' : 'contain');
  };

  const handleScreenClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    setShowControls(prev => !prev);
  };

  const handleLeave = () => {
    syncClient.reset();
    setSession(null);
    setStatus(undefined);
    setError(undefined);
  };



  // 时间显示需要实时从 video 元素读取
  const [, forceUpdate] = useState(0);
  useEffect(() => {
    const interval = setInterval(() => {
      if (videoRef.current && !videoRef.current.paused) {
        forceUpdate(prev => prev + 1);
      }
    }, 500); // 每0.5秒更新一次时间显示
    return () => clearInterval(interval);
  }, []);

  if (isConnected) {
    const currentTime = videoRef.current?.currentTime || 0;
    const duration = videoRef.current?.duration || 0;
    const isPaused = videoRef.current?.paused ?? true;

    return (
      <div className="player-shell dark-theme" onClick={handleScreenClick}>
        <header className={`player-header ${!showControls ? 'hidden' : ''}`} style={{ transition: 'opacity 0.3s', opacity: showControls ? 1 : 0 }}>
          <div className="header-left">
            <span 
              className={`status-dot ${syncClient.connection}`}
              title={`连接状态: ${syncClient.connection}`}
            ></span>
            <span className="room-name">{session.room}</span>
          </div>
          <button 
            className="btn-ghost" 
            onClick={(e) => { 
              e.stopPropagation(); 
              handleLeave(); 
            }}
          >
            退出
          </button>
        </header>

        <div className="video-container" onClick={handleScreenClick}>
          {hasMedia ? (
            <video
              ref={videoRef}
              key={videoUrl}
              className={`main-video ${fitMode === 'cover' ? 'cover-mode' : ''}`}
              playsInline
              poster={coverUrl}
              src={videoUrl}
              preload="auto"
              crossOrigin="anonymous"
              onError={(e) => console.error("Video load error", e)}
            />
          ) : (
            <div className="empty-state">
              <div className="spinner"></div>
              <p>等待房主投送视频...</p>
              <p className="sub-text">请在桌面端选择视频并点击"推送"</p>
            </div>
          )}
        </div>

        {/* 自定义控制层 */}
        {hasMedia && showControls && (
          <div className="controls-layer">
             {/* 中间的大播放按钮 */}
             <div style={{ 
               position: 'absolute', top: '50%', left: '50%', transform: 'translate(-50%, -50%)',
               pointerEvents: 'auto',
               zIndex: 10
             }}>
                <button 
                  className="btn-icon btn-large" 
                  onClick={(e) => { 
                    e.stopPropagation(); 
                    togglePlay(); 
                  }}
                >
                   {isPaused ? '▶' : '⏸'}
                </button>
             </div>

             {/* 底部控制栏 */}
             <div className="bottom-controls" onClick={e => e.stopPropagation()}>
                {/* 进度条和时间信息 */}
                <div className="progress-section">
                  <input 
                    type="range"
                    min="0"
                    max={duration || 100}
                    value={currentTime}
                    step="0.1"
                    className="progress-bar"
                    onChange={(e) => {
                      if (videoRef.current) {
                        videoRef.current.currentTime = parseFloat(e.target.value);
                      }
                    }}
                    onClick={(e) => e.stopPropagation()}
                  />
                  <div className="time-info">
                    <span className="time-code">
                      {formatTime(currentTime)} / {formatTime(duration)}
                    </span>
                  </div>
                </div>

                {/* 按钮行 */}
                <div className="control-row">
                   <div style={{ flex: 1 }}></div>
                   <div style={{ display: 'flex', gap: '16px' }}>
                      <button className="btn-icon" onClick={toggleFitMode}>
                        {fitMode === 'contain' ? '⤢' : '⤡'}
                      </button>
                      <button className="btn-icon" onClick={toggleFullscreen}>
                        {isFullscreen ? '⬓' : '⛶'}
                      </button>
                   </div>
                </div>
             </div>
          </div>
        )}

      </div>
    );
  }

  return (
    <div className="login-shell">
      <header className="login-header">
        <h1>VO Sync</h1>
        <p>与好友同步观看</p>
      </header>

      <form className="login-form" onSubmit={handleJoin}>
        <div className="form-group">
          <label>服务端地址 (Host IP)</label>
          <input
            type="text"
            value={serverUrl}
            onChange={(e) => setServerUrl(e.target.value)}
            placeholder="例如 http://192.168.1.5:18080"
            required
          />
          <p className="hint">请填写 BiliTools 桌面端显示的地址</p>
        </div>

        <div className="form-row">
          <div className="form-group">
            <label>房间号</label>
            <input
              type="text"
              value={room}
              onChange={(e) => setRoom(e.target.value)}
              required
            />
          </div>
          <div className="form-group">
            <label>口令</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
            />
          </div>
        </div>

        <button type="submit" className="btn-primary btn-block">
          加入房间
        </button>

        {status && <div className="msg info">{status}</div>}
        {error && <div className="msg error">{error}</div>}
      </form>
    </div>
  );
}

function formatTime(seconds: number) {
  if (!seconds || isNaN(seconds)) return "00:00";
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
}

export default App;
