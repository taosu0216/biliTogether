package rooms

import (
	"os"
	"path/filepath"
	"testing"
)

func TestJoinRoomAndAuthorize(t *testing.T) {
	m := NewManager(t.TempDir())

	hostID, isHost, err := m.JoinRoom("room1", "pwd")
	if err != nil {
		t.Fatalf("join room failed: %v", err)
	}
	if !isHost {
		t.Fatalf("first member should become host")
	}

	memberID, isHost, err := m.JoinRoom("room1", "pwd")
	if err != nil {
		t.Fatalf("second join failed: %v", err)
	}
	if isHost {
		t.Fatalf("second member must not be host")
	}

	if _, _, err = m.JoinRoom("room1", "wrong"); err != ErrWrongPassword {
		t.Fatalf("expected ErrWrongPassword, got %v", err)
	}

	isHostFlag, err := m.Authorize("room1", "pwd", hostID)
	if err != nil {
		t.Fatalf("authorize host failed: %v", err)
	}
	if !isHostFlag {
		t.Fatalf("authorize should mark host")
	}

	isHostFlag, err = m.Authorize("room1", "pwd", memberID)
	if err != nil {
		t.Fatalf("authorize member failed: %v", err)
	}
	if isHostFlag {
		t.Fatalf("member should not be host")
	}
}

func TestUpdateStateHostOnly(t *testing.T) {
	m := NewManager(t.TempDir())
	hostID, _, _ := m.JoinRoom("room", "pwd")
	memberID, _, _ := m.JoinRoom("room", "pwd")

	state := &RoomState{
		URL:          "https://example.com/video",
		CurrentTime:  1.5,
		Duration:     120,
		Paused:       false,
		PlaybackRate: 1.0,
		SourceType:   "web_embed",
	}

	if _, err := m.UpdateState("room", memberID, state); err != ErrNotHost {
		t.Fatalf("member update should be denied, got %v", err)
	}

	updated, err := m.UpdateState("room", hostID, state)
	if err != nil {
		t.Fatalf("host update failed: %v", err)
	}
	if updated.UpdatedAt == 0 {
		t.Fatalf("UpdatedAt should be set")
	}

	current := m.CurrentState("room")
	if current == nil || current.URL != state.URL {
		t.Fatalf("CurrentState mismatch, got %+v", current)
	}
}

func TestResolveMediaPath(t *testing.T) {
	root := t.TempDir()
	videoPath := filepath.Join(root, "video.mp4")
	if err := os.WriteFile(videoPath, []byte("dummy"), 0o644); err != nil {
		t.Fatalf("write temp media failed: %v", err)
	}

	m := NewManager(root)
	hostID, _, _ := m.JoinRoom("room", "pwd")

	token, err := m.ResolveMediaPath("room", hostID, videoPath)
	if err != nil {
		t.Fatalf("resolve media failed: %v", err)
	}
	if token == "" {
		t.Fatalf("token should not be empty")
	}

	resolvedPath, roomName, err := m.OpenMedia(token)
	if err != nil {
		t.Fatalf("open media failed: %v", err)
	}
	if resolvedPath != videoPath || roomName != "room" {
		t.Fatalf("resolved media mismatch")
	}

	outside := filepath.Join(t.TempDir(), "bad.mp4")
	if err := os.WriteFile(outside, []byte("bad"), 0o644); err != nil {
		t.Fatalf("write outside file failed: %v", err)
	}
	if _, err := m.ResolveMediaPath("room", hostID, outside); err != ErrMediaForbidden {
		t.Fatalf("expected ErrMediaForbidden, got %v", err)
	}
}
