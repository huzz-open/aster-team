package filesystem

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"io"
	"os"
	"path/filepath"
	"runtime"

	"aster.local/team/operations/backend/internal/commercial"
)

// PublicCatalogExporter confines all request-derived paths to an opened root.
// Each revision has one complete plans.json; there is no mutable latest pointer.
type PublicCatalogExporter struct{ root *os.Root }

func NewPublicCatalogExporter(directory string) (*PublicCatalogExporter, error) {
	if directory == "" {
		return nil, errors.New("catalog export directory is required")
	}
	absolute, err := filepath.Abs(directory)
	if err != nil {
		return nil, err
	}
	if err := os.MkdirAll(absolute, 0o700); err != nil {
		return nil, err
	}
	resolved, err := filepath.EvalSymlinks(absolute)
	if err != nil {
		return nil, err
	}
	root, err := os.OpenRoot(resolved)
	if err != nil {
		return nil, err
	}
	return &PublicCatalogExporter{root: root}, nil
}
func (s *PublicCatalogExporter) Close() error { return s.root.Close() }

func (s *PublicCatalogExporter) environment(record commercial.CatalogApprovalRecord, create bool) (*os.Root, []byte, error) {
	raw, err := record.PublicBytes()
	if err != nil {
		return nil, nil, err
	}
	environment := record.Snapshot.Request.Environment
	if create {
		if err := s.root.Mkdir(environment, 0o700); err != nil && !errors.Is(err, os.ErrExist) {
			return nil, nil, err
		}
	}
	info, err := s.root.Lstat(environment)
	if err != nil {
		return nil, nil, err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return nil, nil, errors.New("catalog environment must be a real directory")
	}
	root, err := s.root.OpenRoot(environment)
	return root, raw, err
}
func readCatalog(root *os.Root, id string, expected []byte) ([]byte, error) {
	info, err := root.Lstat(id)
	if err != nil {
		return nil, err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return nil, errors.New("catalog revision must be a real directory")
	}
	directory, err := root.OpenRoot(id)
	if err != nil {
		return nil, err
	}
	defer directory.Close()
	info, err = directory.Lstat("plans.json")
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() || info.Size() != int64(len(expected)) {
		return nil, errors.New("catalog export size or file type differs")
	}
	file, err := directory.Open("plans.json")
	if err != nil {
		return nil, err
	}
	defer file.Close()
	raw, err := io.ReadAll(io.LimitReader(file, int64(len(expected))+1))
	if err != nil {
		return nil, err
	}
	if !bytes.Equal(raw, expected) {
		return nil, errors.New("catalog export differs from approved bytes")
	}
	return raw, nil
}
func (s *PublicCatalogExporter) Read(ctx context.Context, record commercial.CatalogApprovalRecord) ([]byte, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	root, raw, err := s.environment(record, false)
	if err != nil {
		return nil, err
	}
	defer root.Close()
	return readCatalog(root, record.Snapshot.ID, raw)
}
func syncCatalogDirectory(root *os.Root) error {
	// Windows does not expose a portable directory fsync. Export/download always
	// verifies the actual file, so a lost entry can be recreated from its immutable
	// approval rather than being trusted from an old database status alone.
	if runtime.GOOS == "windows" {
		return nil
	}
	file, err := root.Open(".")
	if err != nil {
		return err
	}
	defer file.Close()
	return file.Sync()
}
func (s *PublicCatalogExporter) Export(ctx context.Context, record commercial.CatalogApprovalRecord) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	root, raw, err := s.environment(record, true)
	if err != nil {
		return err
	}
	defer root.Close()
	id := record.Snapshot.ID
	repairMissing := false
	if _, err := root.Lstat(id); err == nil {
		_, err = readCatalog(root, id, raw)
		if err == nil {
			return finalizeCatalogExport(root, id, raw)
		}
		if !errors.Is(err, os.ErrNotExist) {
			return err
		}
		repairMissing = true
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	var nonce [16]byte
	if _, err := rand.Read(nonce[:]); err != nil {
		return err
	}
	temporary := ".pending-" + hex.EncodeToString(nonce[:])
	if err := root.Mkdir(temporary, 0o700); err != nil {
		return err
	}
	defer func() { _ = root.Remove(temporary + "/plans.json"); _ = root.Remove(temporary) }()
	stage, err := root.OpenRoot(temporary)
	if err != nil {
		return err
	}
	writeErr := func() error {
		defer stage.Close()
		file, err := stage.OpenFile("plans.json", os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
		if err != nil {
			return err
		}
		if _, err := file.Write(raw); err != nil {
			_ = file.Close()
			return err
		}
		if err := file.Sync(); err != nil {
			_ = file.Close()
			return err
		}
		if err := file.Close(); err != nil {
			return err
		}
		return syncCatalogDirectory(stage)
	}()
	if writeErr != nil {
		return writeErr
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	// Rename a closed, fully populated directory. A competing nonempty revision
	// cannot be overwritten. On failure accept only the exact winner's bytes.
	var publishErr error
	if repairMissing {
		// Link is an atomic create-if-absent: restore only the missing file, never
		// replace unknown bytes, links or files supplied by another writer.
		info, err := root.Lstat(id)
		if err != nil {
			return err
		}
		if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
			return errors.New("catalog revision must be a real directory")
		}
		publishErr = root.Link(temporary+"/plans.json", id+"/plans.json")
	} else {
		publishErr = root.Rename(temporary, id)
	}
	if publishErr != nil {
		if _, readErr := readCatalog(root, id, raw); readErr != nil {
			return errors.Join(publishErr, readErr)
		}
	}
	return finalizeCatalogExport(root, id, raw)
}

func finalizeCatalogExport(root *os.Root, id string, raw []byte) error {
	directory, err := root.OpenRoot(id)
	if err != nil {
		return err
	}
	err = syncCatalogDirectory(directory)
	_ = directory.Close()
	if err != nil {
		return err
	}
	if err := syncCatalogDirectory(root); err != nil {
		return err
	}
	_, err = readCatalog(root, id, raw)
	return err
}
