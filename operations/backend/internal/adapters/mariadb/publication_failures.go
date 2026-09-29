package mariadb

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

func (s *Store) RecordPublicationFailure(ctx context.Context, event commercial.PublicationFailure) error {
	raw, err := event.Bytes()
	if err != nil {
		return err
	}
	created, _ := time.Parse("2006-01-02T15:04:05.000Z", event.CreatedAt)
	return retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		publication, err := scanPublication(tx.QueryRowContext(ctx, publicationSelect+" WHERE id=? FOR UPDATE", event.PublicationID))
		if err != nil {
			return err
		}
		if event.PublicationSHA256 != publication.SHA256 || event.CreatedAt < publication.Snapshot.CreatedAt {
			return commercial.ErrInvalidPublication
		}
		var previous []byte
		var digest string
		err = tx.QueryRowContext(ctx, "SELECT event_json,content_sha256 FROM commercial_publication_failures WHERE id=? FOR UPDATE", event.ID).Scan(&previous, &digest)
		if err == nil {
			if !bytes.Equal(previous, raw) || digest != commercial.ContentDigest(raw) {
				return commercial.ErrConflict
			}
			return tx.Commit()
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if _, err = tx.ExecContext(ctx, "INSERT INTO commercial_publication_failures(id,publication_id,event_json,content_sha256,created_at) VALUES(?,?,?,?,?)", event.ID, event.PublicationID, raw, commercial.ContentDigest(raw), created); err != nil {
			return err
		}
		if err = writeCommercialAudit(ctx, tx, event.ID, "commercial.publication_attempt_failed", event.PublicationID, event.OperatorID, commercial.ContentDigest(raw), created); err != nil {
			return err
		}
		return tx.Commit()
	})
}

func (s *Store) ListPublicationFailures(ctx context.Context, id string, limit int) ([]commercial.PublicationFailure, error) {
	publication, err := s.GetPublication(ctx, id)
	if err != nil {
		return nil, err
	}
	if limit < 1 || limit > 100 {
		limit = 100
	}
	rows, err := s.db.QueryContext(ctx, "SELECT id,event_json,content_sha256,created_at FROM commercial_publication_failures WHERE publication_id=? ORDER BY created_at DESC,id LIMIT ?", id, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []commercial.PublicationFailure{}
	for rows.Next() {
		var event commercial.PublicationFailure
		var storedID, digest string
		var raw []byte
		var created time.Time
		if err := rows.Scan(&storedID, &raw, &digest, &created); err != nil {
			return nil, err
		}
		if err := json.Unmarshal(raw, &event); err != nil {
			return nil, err
		}
		canonical, err := event.Bytes()
		if err != nil || !bytes.Equal(canonical, raw) || commercial.ContentDigest(raw) != digest || event.ID != storedID || event.PublicationID != id || event.PublicationSHA256 != publication.SHA256 || event.CreatedAt != created.UTC().Format("2006-01-02T15:04:05.000Z") || event.CreatedAt < publication.Snapshot.CreatedAt {
			return nil, commercial.ErrInvalidPublication
		}
		result = append(result, event)
	}
	return result, rows.Err()
}
