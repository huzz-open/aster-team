package application

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

type catalogRecoveryStore struct {
	*commercialPermissionStore
	record   commercial.CatalogApprovalRecord
	failMark bool
	marks    int
}

func (s *catalogRecoveryStore) PreviewPublicCatalog(context.Context, string, commercial.CatalogRequest) (commercial.CatalogPreview, error) {
	panic("unexpected preview")
}
func (s *catalogRecoveryStore) ApprovePublicCatalog(context.Context, string, commercial.ApproveCatalogInput, string, time.Time) (commercial.CatalogApprovalRecord, error) {
	panic("unexpected approval")
}
func (s *catalogRecoveryStore) ListCatalogApprovals(context.Context, int) ([]commercial.CatalogApprovalRecord, error) {
	panic("unexpected list")
}
func (s *catalogRecoveryStore) GetCatalogApproval(context.Context, string) (commercial.CatalogApprovalRecord, error) {
	return s.record, nil
}
func (s *catalogRecoveryStore) MarkCatalogExported(_ context.Context, _ string, digest, _ string, _ time.Time) (commercial.CatalogApprovalRecord, error) {
	s.marks++
	if s.failMark {
		s.failMark = false
		return commercial.CatalogApprovalRecord{}, errors.New("database response lost after file write")
	}
	if digest != s.record.Public.SHA256 {
		return commercial.CatalogApprovalRecord{}, errors.New("wrong file acknowledged")
	}
	s.record.Status = "exported"
	return s.record, nil
}

type catalogRecoveryExporter struct {
	exports, reads       int
	failExport, failRead bool
}

func (e *catalogRecoveryExporter) Export(context.Context, commercial.CatalogApprovalRecord) error {
	e.exports++
	if e.failExport {
		return errors.New("write failed")
	}
	return nil
}
func (e *catalogRecoveryExporter) Read(context.Context, commercial.CatalogApprovalRecord) ([]byte, error) {
	e.reads++
	if e.failRead {
		return nil, errors.New("actual file missing")
	}
	return []byte("test public bytes"), nil
}

func TestPublicCatalogExportRechecksFilesAfterDatabaseAndDownloadFailures(t *testing.T) {
	id := "catalog_" + strings.Repeat("a", 48)
	store := &catalogRecoveryStore{commercialPermissionStore: &commercialPermissionStore{fakeStore: &fakeStore{}, allowed: map[string]bool{PermissionCatalogRead: true, PermissionCatalogExport: true}}, record: commercial.CatalogApprovalRecord{Status: "approved", Public: commercial.CatalogPreview{SHA256: strings.Repeat("b", 64)}}, failMark: true}
	exporter := &catalogRecoveryExporter{}
	s := NewService(store, time.Hour, WithPublicCatalogExporter(exporter))
	ctx := context.Background()
	if _, _, err := s.DownloadPublicCatalog(ctx, id, "operator_1"); !errors.Is(err, ErrCatalogNotExported) || exporter.reads != 0 {
		t.Fatal("unexported record downloaded", err)
	}
	if _, err := s.ExportPublicCatalog(ctx, id, "operator_1"); err == nil || exporter.exports != 1 {
		t.Fatal("lost commit response ignored", err)
	}
	if _, err := s.ExportPublicCatalog(ctx, id, "operator_1"); err != nil || exporter.exports != 2 || store.record.Status != "exported" {
		t.Fatal("export did not resume", err)
	}
	exporter.failRead = true
	if _, _, err := s.DownloadPublicCatalog(ctx, id, "operator_1"); err == nil {
		t.Fatal("database status replaced actual file check")
	}
	exporter.failExport = true
	if _, err := s.ExportPublicCatalog(ctx, id, "operator_1"); err == nil || store.marks != 2 {
		t.Fatal("file failure recorded as export", err)
	}
	exporter.failExport = false
	if _, err := s.ExportPublicCatalog(ctx, id, "operator_1"); err != nil || exporter.exports != 4 {
		t.Fatal("exported status skipped repair", err)
	}
	store.allowed[PermissionCatalogExport] = false
	if _, err := s.ExportPublicCatalog(ctx, id, "operator_1"); !errors.Is(err, ErrUnauthorized) || exporter.exports != 4 {
		t.Fatal("repair bypassed permissions", err)
	}
}
