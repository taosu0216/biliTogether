const STORAGE_KEY = 'voSyncBaseUrl';
const DEFAULT_BASE = 'http://127.0.0.1:18080';

export function getSyncBase(): string {
  return localStorage.getItem(STORAGE_KEY) || DEFAULT_BASE;
}

export function setSyncBase(url: string) {
  const normalized = url.trim().replace(/\/+$/, '');
  localStorage.setItem(STORAGE_KEY, normalized || DEFAULT_BASE);
}

export type RoomState = {
  url: string;
  title: string;
  currentTime: number;
  duration: number;
  paused: boolean;
  playbackRate: number;
  sourceType: string;
  updatedAt: number;
};

export type JoinResponse = { tempUser: string; role: 'host' | 'member' };
export type ResolveResponse = {
  token: string;
  url: string;
  expiresAt: number;
  sourceType: string;
};

type MediaRootResponse = { media_root: string | null };

export async function fetchMediaRoot(base = getSyncBase()): Promise<string | null> {
  const res = await fetch(`${base}/api/media/root`);
  if (!res.ok) throw new Error(`GET media root failed: ${res.status}`);
  const body = (await res.json()) as MediaRootResponse;
  return body.media_root;
}

export async function updateMediaRoot(
  path: string,
  base = getSyncBase(),
): Promise<string | null> {
  const res = await fetch(`${base}/api/media/root`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path }),
  });
  if (!res.ok) throw new Error(`SET media root failed: ${res.status}`);
  const body = (await res.json()) as MediaRootResponse;
  return body.media_root;
}

export async function joinRoom(
  room: string,
  password: string,
  base = getSyncBase(),
): Promise<JoinResponse> {
  const res = await fetch(`${base}/api/room/join`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ room, password }),
  });
  if (!res.ok) {
    const msg = await res.text();
    throw new Error(`join failed: ${msg || res.status}`);
  }
  return (await res.json()) as JoinResponse;
}

export async function resolveMedia(
  params: { room: string; password: string; tempUser: string; path: string },
  base = getSyncBase(),
): Promise<ResolveResponse> {
  const res = await fetch(`${base}/api/media/resolve`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(params),
  });
  if (!res.ok) {
    let msg = '';
    try {
      const body = await res.json();
      msg = body?.error || JSON.stringify(body);
    } catch {
      msg = await res.text();
    }
    throw new Error(`resolve failed: ${msg || res.status}`);
  }
  return (await res.json()) as ResolveResponse;
}

export type SyncSocket = {
  close: () => void;
  sendHostUpdate: (state: RoomState) => void;
  sendPing: () => void;
};

export function connectRoom(opts: {
  base?: string;
  room: string;
  password: string;
  tempUser: string;
  isHost: boolean;
  onState?: (state: RoomState) => void;
  onClose?: () => void;
  onError?: (e: any) => void;
}): SyncSocket {
  const base = (opts.base || getSyncBase()).replace(/\/+$/, '');
  const wsUrl = base.replace(/^http/, 'ws');
  const ws = new WebSocket(
    `${wsUrl}/ws?room=${encodeURIComponent(opts.room)}&password=${encodeURIComponent(
      opts.password,
    )}&tempUser=${encodeURIComponent(opts.tempUser)}`,
  );
  let pingTimer: number | undefined;

  ws.onopen = () => {
    pingTimer = window.setInterval(() => {
      ws.send(JSON.stringify({ type: 'member_ping', tempUser: opts.tempUser }));
    }, 20000);
  };

  ws.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      if (data.type === 'room_state' && data.state) {
        opts.onState?.(data.state as RoomState);
      }
    } catch (err) {
      opts.onError?.(err);
    }
  };

  ws.onclose = () => {
    if (pingTimer) window.clearInterval(pingTimer);
    opts.onClose?.();
  };
  ws.onerror = async (e) => {
    try {
      opts.onError?.(new Error(`ws connect error (${ws.url}): ${e.type}`));
    } catch {
      opts.onError?.(e);
    }
  };

  return {
    close: () => {
      if (pingTimer) window.clearInterval(pingTimer);
      ws.close();
    },
    sendHostUpdate: (state: RoomState) => {
      ws.send(JSON.stringify({ type: 'host_update', state }));
    },
    sendPing: () => ws.send(JSON.stringify({ type: 'member_ping' })),
  };
}
