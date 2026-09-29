package filesystem

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	"aster.local/team/operations/backend/internal/ports"
)

var sha256Pattern = regexp.MustCompile(`^[0-9a-f]{64}$`)
var objectKeyPattern = regexp.MustCompile(`^objects/([0-9a-f]{2})/([0-9a-f]{64})$`)
var cacheKeyPattern = regexp.MustCompile(`^cache/([0-9a-f]{2})/([0-9a-f]{64})$`)

type ArtifactStore struct {
	root    string
	inbox   string
	objects string
	cache   string
}

var _ ports.ArtifactStore = (*ArtifactStore)(nil)

func NewArtifactStore(root string) (*ArtifactStore, error) {
	absolute, err := filepath.Abs(root)
	if err != nil {
		return nil, err
	}
	store := &ArtifactStore{root: absolute, inbox: filepath.Join(absolute, "inbox"), objects: filepath.Join(absolute, "objects"), cache: filepath.Join(absolute, "cache")}
	for _, directory := range []string{store.root, store.inbox, store.objects, store.cache} {
		if err := os.MkdirAll(directory, 0o750); err != nil {
			return nil, fmt.Errorf("create artifact directory: %w", err)
		}
	}
	return store, nil
}

func (store *ArtifactStore) ImportInbox(ctx context.Context, filename, expectedSHA256 string) (ports.StoredArtifact, error) {
	if filename == "" || filepath.Base(filename) != filename || strings.ContainsAny(filename, `/\`) {
		return ports.StoredArtifact{}, errors.New("inbox filename must be a plain filename")
	}
	if !sha256Pattern.MatchString(expectedSHA256) {
		return ports.StoredArtifact{}, errors.New("expected SHA-256 is invalid")
	}
	source := filepath.Join(store.inbox, filename)
	info, err := os.Lstat(source)
	if err != nil {
		return ports.StoredArtifact{}, fmt.Errorf("inspect inbox artifact: %w", err)
	}
	if info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
		return ports.StoredArtifact{}, errors.New("inbox artifact must be a regular non-symlink file")
	}
	file, err := os.Open(source)
	if err != nil {
		return ports.StoredArtifact{}, err
	}
	defer file.Close()
	return store.put(ctx, "objects", store.objects, file, expectedSHA256, info.Size())
}

func (store *ArtifactStore) PutObject(ctx context.Context, source io.Reader, expectedSHA256 string, maximumBytes int64) (ports.StoredArtifact, error) {
	if source == nil || !sha256Pattern.MatchString(expectedSHA256) || maximumBytes < 1 {
		return ports.StoredArtifact{}, errors.New("artifact object input is invalid")
	}
	return store.put(ctx, "objects", store.objects, source, expectedSHA256, maximumBytes)
}

func (store *ArtifactStore) PutCachedObject(ctx context.Context, source io.Reader, expectedSHA256 string, maximumBytes int64) (ports.StoredArtifact, error) {
	if source == nil || !sha256Pattern.MatchString(expectedSHA256) || maximumBytes < 1 {
		return ports.StoredArtifact{}, errors.New("cached artifact input is invalid")
	}
	return store.put(ctx, "cache", store.cache, source, expectedSHA256, maximumBytes)
}

func (store *ArtifactStore) PutReceipt(ctx context.Context, contents []byte) (ports.StoredArtifact, error) {
	digest := sha256.Sum256(contents)
	return store.put(ctx, "objects", store.objects, strings.NewReader(string(contents)), hex.EncodeToString(digest[:]), int64(len(contents)))
}

func (store *ArtifactStore) ReadObject(ctx context.Context, objectKey string, maximumBytes int64) ([]byte, error) {
	file, _, err := store.OpenObject(ctx, objectKey, maximumBytes)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	return io.ReadAll(file)
}

func (store *ArtifactStore) OpenObject(ctx context.Context, objectKey string, maximumBytes int64) (ports.ReadSeekCloser, int64, error) {
	return store.open(ctx, objectKey, objectKeyPattern, maximumBytes)
}

func (store *ArtifactStore) OpenCachedObject(ctx context.Context, expectedSHA256 string, maximumBytes int64) (ports.ReadSeekCloser, int64, error) {
	if !sha256Pattern.MatchString(expectedSHA256) {
		return nil, 0, errors.New("cached artifact SHA-256 is invalid")
	}
	objectKey := filepath.ToSlash(filepath.Join("cache", expectedSHA256[:2], expectedSHA256))
	return store.open(ctx, objectKey, cacheKeyPattern, maximumBytes)
}

func (store *ArtifactStore) open(ctx context.Context, objectKey string, pattern *regexp.Regexp, maximumBytes int64) (ports.ReadSeekCloser, int64, error) {
	match := pattern.FindStringSubmatch(filepath.ToSlash(objectKey))
	if len(match) != 3 || match[1] != match[2][:2] || maximumBytes < 1 {
		return nil, 0, errors.New("artifact object key is invalid")
	}
	path := filepath.Join(store.root, filepath.FromSlash(objectKey))
	info, err := os.Lstat(path)
	if err != nil || !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 || info.Size() > maximumBytes {
		return nil, 0, errors.New("artifact object is unavailable or exceeds the size limit")
	}
	if err := ctx.Err(); err != nil {
		return nil, 0, err
	}
	file, err := os.Open(path)
	if err != nil {
		return nil, 0, err
	}
	digest := sha256.New()
	if _, err := io.Copy(digest, io.LimitReader(&contextReader{ctx: ctx, reader: file}, maximumBytes+1)); err != nil {
		file.Close()
		return nil, 0, err
	}
	if hex.EncodeToString(digest.Sum(nil)) != match[2] {
		file.Close()
		return nil, 0, errors.New("artifact object hash does not match its key")
	}
	if _, err := file.Seek(0, io.SeekStart); err != nil {
		file.Close()
		return nil, 0, err
	}
	return file, info.Size(), nil
}

func (store *ArtifactStore) put(ctx context.Context, namespace, root string, source io.Reader, expectedSHA256 string, maximumBytes int64) (ports.StoredArtifact, error) {
	if err := ctx.Err(); err != nil {
		return ports.StoredArtifact{}, err
	}
	if maximumBytes < 1 {
		return ports.StoredArtifact{}, errors.New("artifact maximum size is invalid")
	}
	directory := filepath.Join(root, expectedSHA256[:2])
	if err := os.MkdirAll(directory, 0o750); err != nil {
		return ports.StoredArtifact{}, err
	}
	destination := filepath.Join(directory, expectedSHA256)
	if existing, err := os.Stat(destination); err == nil && existing.Mode().IsRegular() {
		objectKey := filepath.ToSlash(filepath.Join(namespace, expectedSHA256[:2], expectedSHA256))
		pattern := objectKeyPattern
		if namespace == "cache" {
			pattern = cacheKeyPattern
		}
		opened, size, openErr := store.open(ctx, objectKey, pattern, maximumBytes)
		if openErr == nil {
			opened.Close()
			return ports.StoredArtifact{ObjectKey: objectKey, SHA256: expectedSHA256, SizeBytes: size}, nil
		}
		if err := os.Remove(destination); err != nil {
			return ports.StoredArtifact{}, fmt.Errorf("replace invalid artifact object: %w", err)
		}
	}
	temporary, err := os.CreateTemp(directory, ".ingest-*")
	if err != nil {
		return ports.StoredArtifact{}, err
	}
	temporaryName := temporary.Name()
	defer os.Remove(temporaryName)
	hash := sha256.New()
	written, copyErr := io.Copy(io.MultiWriter(temporary, hash), io.LimitReader(&contextReader{ctx: ctx, reader: source}, maximumBytes+1))
	closeErr := temporary.Close()
	if copyErr != nil {
		return ports.StoredArtifact{}, copyErr
	}
	if closeErr != nil {
		return ports.StoredArtifact{}, closeErr
	}
	if written > maximumBytes {
		return ports.StoredArtifact{}, errors.New("artifact exceeds the size limit")
	}
	actual := hex.EncodeToString(hash.Sum(nil))
	if actual != expectedSHA256 {
		return ports.StoredArtifact{}, fmt.Errorf("artifact SHA-256 mismatch: expected %s, got %s", expectedSHA256, actual)
	}
	if err := os.Chmod(temporaryName, 0o640); err != nil {
		return ports.StoredArtifact{}, err
	}
	if err := os.Rename(temporaryName, destination); err != nil {
		objectKey := filepath.ToSlash(filepath.Join(namespace, expectedSHA256[:2], expectedSHA256))
		pattern := objectKeyPattern
		if namespace == "cache" {
			pattern = cacheKeyPattern
		}
		opened, size, openErr := store.open(ctx, objectKey, pattern, maximumBytes)
		if openErr != nil {
			return ports.StoredArtifact{}, err
		}
		opened.Close()
		return ports.StoredArtifact{ObjectKey: objectKey, SHA256: expectedSHA256, SizeBytes: size}, nil
	}
	return ports.StoredArtifact{ObjectKey: filepath.ToSlash(filepath.Join(namespace, expectedSHA256[:2], expectedSHA256)), SHA256: expectedSHA256, SizeBytes: written}, nil
}

type contextReader struct {
	ctx    context.Context
	reader io.Reader
}

func (reader *contextReader) Read(buffer []byte) (int, error) {
	if err := reader.ctx.Err(); err != nil {
		return 0, err
	}
	return reader.reader.Read(buffer)
}
