package rooms

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
)

var (
	ErrRoomNotFound   = errors.New("room not found")
	ErrWrongPassword  = errors.New("room password mismatch")
	ErrNotHost        = errors.New("operation allowed for host only")
	ErrMediaForbidden = errors.New("media path forbidden")
)

// RoomState 描述一次同步状态
type RoomState struct {
	URL          string  `json:"url"`
	Title        string  `json:"title"`
	CurrentTime  float64 `json:"currentTime"`
	Duration     float64 `json:"duration"`
	Paused       bool    `json:"paused"`
	PlaybackRate float64 `json:"playbackRate"`
	SourceType   string  `json:"sourceType"`
	UpdatedAt    int64   `json:"updatedAt"`
}

type room struct {
	Name       string
	Password   string
	HostID     string
	State      *RoomState
	Members    map[string]time.Time
	LastUpdate time.Time
}

type mediaToken struct {
	Token     string
	Path      string
	RoomName  string
	ExpiresAt time.Time
}

// Manager 负责房间、成员、媒体的状态管理
type Manager struct {
	mu          sync.RWMutex
	rooms       map[string]*room
	mediaTokens map[string]*mediaToken
	mediaRoot   string
	roomTTL     time.Duration
	tokenTTL    time.Duration
}

func NewManager(mediaRoot string) *Manager {
	m := &Manager{
		rooms:       make(map[string]*room),
		mediaTokens: make(map[string]*mediaToken),
		mediaRoot:   filepath.Clean(mediaRoot),
		roomTTL:     30 * time.Minute,
		tokenTTL:    1 * time.Hour,
	}
	go m.cleanupLoop()
	return m
}

// JoinRoom 返回新生成的临时用户 ID 以及是否成为房主
func (m *Manager) JoinRoom(name, password string) (tempUser string, isHost bool, err error) {
	name = strings.TrimSpace(name)
	password = strings.TrimSpace(password)
	if name == "" || password == "" {
		return "", false, errors.New("room name and password required")
	}

	tempUser = uuid.NewString()
	m.mu.Lock()
	defer m.mu.Unlock()

	r, ok := m.rooms[name]
	if !ok {
		r = &room{
			Name:     name,
			Password: password,
			HostID:   tempUser,
			Members:  map[string]time.Time{},
		}
		m.rooms[name] = r
		isHost = true
	} else {
		if r.Password != password {
			return "", false, ErrWrongPassword
		}
		if r.HostID == "" {
			r.HostID = tempUser
			isHost = true
		}
	}
	r.Members[tempUser] = time.Now()
	return tempUser, isHost, nil
}

func (m *Manager) Authorize(roomName, password, tempUser string) (isHost bool, err error) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	r, ok := m.rooms[roomName]
	if !ok {
		return false, ErrRoomNotFound
	}
	if r.Password != password {
		return false, ErrWrongPassword
	}
	_, exists := r.Members[tempUser]
	if !exists {
		return false, fmt.Errorf("user %s not in room", tempUser)
	}
	return r.HostID == tempUser, nil
}

func (m *Manager) TouchMember(roomName, tempUser string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if r, ok := m.rooms[roomName]; ok {
		r.Members[tempUser] = time.Now()
	}
}

func (m *Manager) UpdateState(roomName, tempUser string, state *RoomState) (*RoomState, error) {
	if state == nil {
		return nil, errors.New("state required")
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	r, ok := m.rooms[roomName]
	if !ok {
		return nil, ErrRoomNotFound
	}
	if r.HostID != tempUser {
		return nil, ErrNotHost
	}
	stateCopy := *state
	stateCopy.UpdatedAt = time.Now().UnixMilli()
	r.State = &stateCopy
	r.LastUpdate = time.Now()
	return r.State, nil
}

func (m *Manager) CurrentState(roomName string) *RoomState {
	m.mu.RLock()
	defer m.mu.RUnlock()
	if r, ok := m.rooms[roomName]; ok && r.State != nil {
		stateCopy := *r.State
		return &stateCopy
	}
	return nil
}

func (m *Manager) ResolveMediaPath(roomName, tempUser, absPath string) (token string, err error) {
	root := m.mediaRoot
	if root == "" {
		return "", errors.New("media root not configured")
	}
	cleanPath := filepath.Clean(absPath)
	root = filepath.Clean(root)
	if !strings.HasPrefix(cleanPath, root) {
		return "", ErrMediaForbidden
	}
	info, err := os.Stat(cleanPath)
	if err != nil {
		return "", err
	}
	if info.IsDir() {
		return "", errors.New("path is directory")
	}

	m.mu.Lock()
	defer m.mu.Unlock()
	r, ok := m.rooms[roomName]
	if !ok {
		return "", ErrRoomNotFound
	}
	if r.HostID != tempUser {
		return "", ErrNotHost
	}

	token = uuid.NewString()
	m.mediaTokens[token] = &mediaToken{
		Token:     token,
		Path:      cleanPath,
		RoomName:  roomName,
		ExpiresAt: time.Now().Add(m.tokenTTL),
	}
	return token, nil
}

func (m *Manager) OpenMedia(token string) (path string, roomName string, err error) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	t, ok := m.mediaTokens[token]
	if !ok {
		return "", "", errors.New("token not found")
	}
	if time.Now().After(t.ExpiresAt) {
		return "", "", errors.New("token expired")
	}
	return t.Path, t.RoomName, nil
}

func (m *Manager) cleanupLoop() {
	ticker := time.NewTicker(1 * time.Minute)
	defer ticker.Stop()
	for range ticker.C {
		m.cleanup()
	}
}

func (m *Manager) cleanup() {
	m.mu.Lock()
	defer m.mu.Unlock()
	now := time.Now()
	for name, r := range m.rooms {
		lastSeen := r.LastUpdate
		for _, t := range r.Members {
			if t.After(lastSeen) {
				lastSeen = t
			}
		}
		if now.Sub(lastSeen) > m.roomTTL {
			delete(m.rooms, name)
		}
	}
	for token, mt := range m.mediaTokens {
		if now.After(mt.ExpiresAt) {
			delete(m.mediaTokens, token)
		}
	}
}
