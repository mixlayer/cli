package store

import (
	"os"
	"path/filepath"
	"testing"
)

func TestSQLiteFilesArePrivateAtCustomDBPath(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "custom.db")
	s, err := Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{dbPath, dbPath + "-wal", dbPath + "-shm"} {
		info, err := os.Stat(name)
		if os.IsNotExist(err) {
			if name != dbPath {
				continue // SQLite may remove an unused sidecar.
			}
			t.Fatal(err)
		}
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm() != 0o600 {
			t.Fatalf("%s mode = %04o", name, info.Mode().Perm())
		}
	}
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(dbPath, 0o644); err != nil {
		t.Fatal(err)
	}
	reopened, err := Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	info, err := os.Stat(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("reopened mode=%v", info.Mode().Perm())
	}
}
