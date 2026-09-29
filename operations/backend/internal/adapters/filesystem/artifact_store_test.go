package filesystem

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestImportInboxVerifiesHashAndUsesContentAddressedObject(t *testing.T) {
	root := t.TempDir()
	store, err := NewArtifactStore(root)
	if err != nil {
		t.Fatal(err)
	}
	contents := []byte("signed release fixture")
	digest := sha256.Sum256(contents)
	expected := hex.EncodeToString(digest[:])
	if err := os.WriteFile(filepath.Join(root, "inbox", "release.tar.gz"), contents, 0o640); err != nil {
		t.Fatal(err)
	}
	stored, err := store.ImportInbox(context.Background(), "release.tar.gz", expected)
	if err != nil {
		t.Fatal(err)
	}
	if stored.SHA256 != expected || stored.SizeBytes != int64(len(contents)) {
		t.Fatalf("unexpected stored artifact: %#v", stored)
	}
	object, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(stored.ObjectKey)))
	if err != nil {
		t.Fatal(err)
	}
	if string(object) != string(contents) {
		t.Fatal("stored object contents changed")
	}
	read, err := store.ReadObject(context.Background(), stored.ObjectKey, 1024)
	if err != nil || string(read) != string(contents) {
		t.Fatalf("ReadObject() = %q, %v", read, err)
	}
	if _, err := store.ReadObject(context.Background(), "../inbox/release.tar.gz", 1024); err == nil {
		t.Fatal("object path traversal was accepted")
	}
	if _, err := store.ReadObject(context.Background(), stored.ObjectKey, 1); err == nil {
		t.Fatal("object size limit was ignored")
	}
	if _, err := store.ImportInbox(context.Background(), "../release.tar.gz", expected); err == nil {
		t.Fatal("path traversal was accepted")
	}
	if _, err := store.ImportInbox(context.Background(), "release.tar.gz", string(make([]byte, 64))); err == nil {
		t.Fatal("malformed hash was accepted")
	}
}

func TestPutObjectEnforcesDigestAndSizeLimit(t *testing.T) {
	store, err := NewArtifactStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	contents := []byte("verified archive")
	digest := sha256.Sum256(contents)
	expected := hex.EncodeToString(digest[:])
	stored, err := store.PutObject(context.Background(), bytes.NewReader(contents), expected, int64(len(contents)))
	if err != nil || stored.SHA256 != expected {
		t.Fatalf("PutObject() = %#v, %v", stored, err)
	}
	if _, err := store.PutObject(context.Background(), bytes.NewReader(contents), expected, int64(len(contents)-1)); err == nil {
		t.Fatal("PutObject accepted an object above the size limit")
	}
	if _, err := store.PutObject(context.Background(), bytes.NewReader(contents), strings.Repeat("0", 64), int64(len(contents))); err == nil {
		t.Fatal("PutObject accepted a mismatched digest")
	}
}

func TestCachedObjectIsIsolatedReusedAndRehashed(t *testing.T) {
	root := t.TempDir()
	store, err := NewArtifactStore(root)
	if err != nil {
		t.Fatal(err)
	}
	contents := []byte("github artifact zip")
	digest := sha256.Sum256(contents)
	expected := hex.EncodeToString(digest[:])
	stored, err := store.PutCachedObject(context.Background(), bytes.NewReader(contents), expected, int64(len(contents)))
	if err != nil || stored.ObjectKey != filepath.ToSlash(filepath.Join("cache", expected[:2], expected)) {
		t.Fatalf("PutCachedObject() = %#v, %v", stored, err)
	}
	opened, size, err := store.OpenCachedObject(context.Background(), expected, int64(len(contents)))
	if err != nil || size != int64(len(contents)) {
		t.Fatalf("OpenCachedObject() size = %d, %v", size, err)
	}
	read, readErr := io.ReadAll(opened)
	opened.Close()
	if readErr != nil || !bytes.Equal(read, contents) {
		t.Fatalf("cached contents = %q, %v", read, readErr)
	}
	cachePath := filepath.Join(root, filepath.FromSlash(stored.ObjectKey))
	if err := os.WriteFile(cachePath, []byte("corrupt"), 0o640); err != nil {
		t.Fatal(err)
	}
	if _, _, err := store.OpenCachedObject(context.Background(), expected, int64(len(contents))); err == nil {
		t.Fatal("OpenCachedObject accepted a corrupted cache entry")
	}
	restored, err := store.PutCachedObject(context.Background(), bytes.NewReader(contents), expected, int64(len(contents)))
	if err != nil || restored.SHA256 != expected {
		t.Fatalf("PutCachedObject() did not replace corruption: %#v, %v", restored, err)
	}
}
