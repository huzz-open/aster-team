package ports

import (
	"context"
	"io"
)

type StoredArtifact struct {
	ObjectKey string
	SHA256    string
	SizeBytes int64
}

type ReadSeekCloser interface {
	io.Reader
	io.ReaderAt
	io.Seeker
	io.Closer
}

type ArtifactStore interface {
	ImportInbox(context.Context, string, string) (StoredArtifact, error)
	PutObject(context.Context, io.Reader, string, int64) (StoredArtifact, error)
	PutCachedObject(context.Context, io.Reader, string, int64) (StoredArtifact, error)
	PutReceipt(context.Context, []byte) (StoredArtifact, error)
	OpenObject(context.Context, string, int64) (ReadSeekCloser, int64, error)
	OpenCachedObject(context.Context, string, int64) (ReadSeekCloser, int64, error)
	ReadObject(context.Context, string, int64) ([]byte, error)
}
