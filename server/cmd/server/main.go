package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"log"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"

	"github.com/taosu/vo/server/internal/rooms"
)

var upgrader = websocket.Upgrader{
	CheckOrigin: func(r *http.Request) bool { return true },
}

type joinRequest struct {
	Room     string `json:"room"`
	Password string `json:"password"`
}

type joinResponse struct {
	TempUser string `json:"tempUser"`
	Role     string `json:"role"`
}

type mediaResolveRequest struct {
	Room     string `json:"room"`
	Password string `json:"password"`
	TempUser string `json:"tempUser"`
	Path     string `json:"path"`
}

type mediaResolveResponse struct {
	Token     string `json:"token"`
	URL       string `json:"url"`
	ExpiresAt int64  `json:"expiresAt"`
}

type wsIncoming struct {
	Type  string           `json:"type"`
	State *rooms.RoomState `json:"state,omitempty"`
}

type wsOutgoing struct {
	Type  string           `json:"type"`
	State *rooms.RoomState `json:"state,omitempty"`
	Error string           `json:"error,omitempty"`
}

func main() {
	var (
		addr      = flag.String("addr", envOrDefault("SERVER_ADDR", ":8080"), "http listen address")
		mediaRoot = flag.String("media_root", os.Getenv("MEDIA_ROOT"), "media root directory")
	)
	flag.Parse()

	manager := rooms.NewManager(*mediaRoot)
	hub := NewHub(manager)

	mux := http.NewServeMux()
	mux.Handle("/healthz", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok"))
	}))
	mux.Handle("/api/room/join", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			methodNotAllowed(w)
			return
		}
		var req joinRequest
		if err := decodeJSON(r, &req); err != nil {
			writeError(w, http.StatusBadRequest, err)
			return
		}
		tempUser, isHost, err := manager.JoinRoom(req.Room, req.Password)
		if err != nil {
			writeError(w, http.StatusBadRequest, err)
			return
		}
		resp := joinResponse{
			TempUser: tempUser,
			Role:     map[bool]string{true: "host", false: "member"}[isHost],
		}
		writeJSON(w, http.StatusOK, resp)
	}))
	mux.Handle("/api/media/resolve", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			methodNotAllowed(w)
			return
		}
		var req mediaResolveRequest
		if err := decodeJSON(r, &req); err != nil {
			writeError(w, http.StatusBadRequest, err)
			return
		}
		if req.Path == "" {
			writeError(w, http.StatusBadRequest, errors.New("path required"))
			return
		}
		token, err := manager.ResolveMediaPath(req.Room, req.TempUser, req.Path)
		if err != nil {
			writeError(w, http.StatusBadRequest, err)
			return
		}
		resp := mediaResolveResponse{
			Token:     token,
			URL:       "/media/" + token,
			ExpiresAt: time.Now().Add(time.Hour).UnixMilli(),
		}
		writeJSON(w, http.StatusOK, resp)
	}))
	mux.Handle("/media/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		token := strings.TrimPrefix(r.URL.Path, "/media/")
		if token == "" {
			http.NotFound(w, r)
			return
		}
		path, _, err := manager.OpenMedia(token)
		if err != nil {
			http.NotFound(w, r)
			return
		}
		http.ServeFile(w, r, path)
	}))
	mux.Handle("/ws", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		roomName := r.URL.Query().Get("room")
		password := r.URL.Query().Get("password")
		tempUser := r.URL.Query().Get("tempUser")
		if roomName == "" || password == "" || tempUser == "" {
			http.Error(w, "room, password, tempUser required", http.StatusBadRequest)
			return
		}
		isHost, err := manager.Authorize(roomName, password, tempUser)
		if err != nil {
			http.Error(w, err.Error(), http.StatusForbidden)
			return
		}
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			log.Println("upgrade error:", err)
			return
		}
		client := &Client{
			conn:     conn,
			hub:      hub,
			roomName: roomName,
			tempUser: tempUser,
			isHost:   isHost,
		}
		hub.AddClient(client)
		if state := manager.CurrentState(roomName); state != nil {
			client.sendJSON(wsOutgoing{Type: "room_state", State: state})
		}
		go client.readLoop()
	}))

	log.Printf("mobile sync server listening on %s", *addr)
	if err := http.ListenAndServe(*addr, corsMiddleware(mux)); err != nil {
		log.Fatal(err)
	}
}

func envOrDefault(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func decodeJSON(r *http.Request, v interface{}) error {
	defer r.Body.Close()
	decoder := json.NewDecoder(r.Body)
	decoder.DisallowUnknownFields()
	return decoder.Decode(v)
}

func writeJSON(w http.ResponseWriter, status int, v interface{}) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(v); err != nil {
		log.Println("write json error:", err)
	}
}

func writeError(w http.ResponseWriter, status int, err error) {
	writeJSON(w, status, map[string]string{"error": err.Error()})
}

func methodNotAllowed(w http.ResponseWriter) {
	w.WriteHeader(http.StatusMethodNotAllowed)
}

type Hub struct {
	manager *rooms.Manager
	mu      sync.RWMutex
	clients map[string]map[*Client]struct{}
}

func NewHub(manager *rooms.Manager) *Hub {
	return &Hub{
		manager: manager,
		clients: make(map[string]map[*Client]struct{}),
	}
}

func (h *Hub) AddClient(c *Client) {
	h.mu.Lock()
	defer h.mu.Unlock()
	roomClients := h.clients[c.roomName]
	if roomClients == nil {
		roomClients = make(map[*Client]struct{})
		h.clients[c.roomName] = roomClients
	}
	roomClients[c] = struct{}{}
}

func (h *Hub) removeClient(c *Client) {
	h.mu.Lock()
	defer h.mu.Unlock()
	if roomClients, ok := h.clients[c.roomName]; ok {
		delete(roomClients, c)
		if len(roomClients) == 0 {
			delete(h.clients, c.roomName)
		}
	}
}

func (h *Hub) broadcastState(roomName string, state *rooms.RoomState) {
	if state == nil {
		return
	}
	stateCopy := cloneState(state)
	payload, err := json.Marshal(wsOutgoing{
		Type:  "room_state",
		State: stateCopy,
	})
	if err != nil {
		log.Println("broadcast marshal error:", err)
		return
	}
	h.mu.RLock()
	defer h.mu.RUnlock()
	for client := range h.clients[roomName] {
		client.send(payload)
	}
}

type Client struct {
	conn     *websocket.Conn
	hub      *Hub
	roomName string
	tempUser string
	isHost   bool

	writeMu sync.Mutex
}

func (c *Client) readLoop() {
	defer func() {
		c.hub.removeClient(c)
		c.conn.Close()
	}()
	for {
		var msg wsIncoming
		if err := c.conn.ReadJSON(&msg); err != nil {
			if !websocket.IsCloseError(err, websocket.CloseGoingAway, websocket.CloseNormalClosure) {
				log.Println("read error:", err)
			}
			return
		}
		switch msg.Type {
		case "host_update":
			if !c.isHost {
				c.sendJSON(wsOutgoing{Type: "error", Error: "only host can update"})
				continue
			}
			state, err := c.hub.manager.UpdateState(c.roomName, c.tempUser, msg.State)
			if err != nil {
				c.sendJSON(wsOutgoing{Type: "error", Error: err.Error()})
				continue
			}
			c.hub.broadcastState(c.roomName, state)
		case "member_ping":
			c.hub.manager.TouchMember(c.roomName, c.tempUser)
		default:
			c.sendJSON(wsOutgoing{Type: "error", Error: "unknown message type"})
		}
	}
}

func (c *Client) sendJSON(msg wsOutgoing) {
	payload, err := json.Marshal(msg)
	if err != nil {
		log.Println("marshal error:", err)
		return
	}
	c.send(payload)
}

func (c *Client) send(payload []byte) {
	c.writeMu.Lock()
	defer c.writeMu.Unlock()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	deadline, _ := ctx.Deadline()
	c.conn.SetWriteDeadline(deadline)
	if err := c.conn.WriteMessage(websocket.TextMessage, payload); err != nil {
		log.Println("write message error:", err)
	}
}

func cloneState(state *rooms.RoomState) *rooms.RoomState {
	if state == nil {
		return nil
	}
	copy := *state
	return &copy
}

func corsMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Access-Control-Allow-Origin", "*")
		w.Header().Set("Access-Control-Allow-Headers", "Content-Type, Authorization")
		w.Header().Set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
		if r.Method == http.MethodOptions {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		next.ServeHTTP(w, r)
	})
}
