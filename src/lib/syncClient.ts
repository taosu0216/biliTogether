import { useCallback, useEffect, useRef, useState } from "react";

export type Role = "host" | "member";

export interface SyncSession {
  serverUrl: string;
  room: string;
  password: string;
  tempUser: string;
  role: Role;
}

export interface RoomStatePayload {
  url: string;
  title: string;
  currentTime: number;
  duration: number;
  paused: boolean;
  playbackRate: number;
  sourceType: string;
  updatedAt?: number;
  cover?: string;
}

export type ConnectionState = "idle" | "connecting" | "open" | "closed" | "error";

export interface SyncClient {
  state: RoomStatePayload | null;
  connection: ConnectionState;
  lastError?: string;
  lastMessage?: string;
  sendHostUpdate: (state: RoomStatePayload) => void;
  sendMemberPing: () => void;
  reset: () => void;
}

function toWsUrl(session: SyncSession): string {
  const base = new URL(session.serverUrl);
  base.protocol = base.protocol === "https:" ? "wss:" : "ws:";
  base.pathname = "/ws";
  base.search = new URLSearchParams({
    room: session.room,
    password: session.password,
    tempUser: session.tempUser,
  }).toString();
  return base.toString();
}

export function useSyncClient(session: SyncSession | null): SyncClient {
  const socketRef = useRef<WebSocket | null>(null);
  const pingTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const [connection, setConnection] = useState<ConnectionState>("idle");
  const [state, setState] = useState<RoomStatePayload | null>(null);
  const [lastError, setLastError] = useState<string>();
  const [lastMessage, setLastMessage] = useState<string>();

  const cleanup = useCallback(() => {
    if (pingTimer.current) {
      clearInterval(pingTimer.current);
      pingTimer.current = null;
    }
    if (socketRef.current) {
      socketRef.current.close();
      socketRef.current = null;
    }
  }, []);

  useEffect(() => {
    if (!session) {
      cleanup();
      setConnection("idle");
      setState(null);
      return;
    }
    cleanup();
    setConnection("connecting");
    setLastError(undefined);
    try {
      const ws = new WebSocket(toWsUrl(session));
      socketRef.current = ws;
      ws.onopen = () => {
        setConnection("open");
        if (session.role !== "host") {
          ws.send(JSON.stringify({ type: "member_ping" }));
        }
        pingTimer.current = setInterval(() => {
          ws.send(JSON.stringify({ type: "member_ping" }));
        }, 4000);
      };
      ws.onmessage = (event) => {
        try {
          const payload = JSON.parse(event.data);
          console.log('🔔 WebSocket message:', payload.type, payload);
          setLastMessage(JSON.stringify(payload).slice(0, 200)); // Keep it short
          if (payload.type === "room_state" && payload.state) {
            console.log('✅ Setting room state:', payload.state);
            setState(payload.state as RoomStatePayload);
          } else {
            console.log('⚠️ Unknown message type:', payload.type);
          }
        } catch (err) {
          console.warn("ws parse error", err);
          setLastMessage("Parse error: " + event.data);
        }
      };
      ws.onerror = (event) => {
        console.error("ws error", event);
        setConnection("error");
        setLastError("WebSocket 连接错误");
      };
      ws.onclose = () => {
        setConnection("closed");
        cleanup();
      };
    } catch (err) {
      console.error(err);
      setConnection("error");
      setLastError((err as Error).message);
    }
    return cleanup;
  }, [session, cleanup]);

  const sendHostUpdate = useCallback(
    (nextState: RoomStatePayload) => {
      if (!socketRef.current || socketRef.current.readyState !== WebSocket.OPEN) {
        console.warn("ws not ready");
        return;
      }
      socketRef.current.send(
        JSON.stringify({
          type: "host_update",
          state: nextState,
        }),
      );
    },
    [],
  );

  const sendMemberPing = useCallback(() => {
    if (!socketRef.current || socketRef.current.readyState !== WebSocket.OPEN) {
      return;
    }
    socketRef.current.send(JSON.stringify({ type: "member_ping" }));
  }, []);

  return {
    state,
    connection,
    lastError,
    lastMessage,
    sendHostUpdate,
    sendMemberPing,
    reset: cleanup,
  };
}

