package application

import (
	"context"
	"errors"
	"fmt"
	"regexp"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

const PermissionCatalogRead = "commercial.catalog.read"
const PermissionCatalogApprove = "commercial.catalog.approve"
const PermissionCatalogExport = "commercial.catalog.export"

var ErrCatalogExporterUnavailable = errors.New("public catalog exporter is not configured")
var ErrCatalogNotExported = errors.New("public catalog has not been exported")

type PublicCatalogExporter interface {
	Export(context.Context, commercial.CatalogApprovalRecord) error
	Read(context.Context, commercial.CatalogApprovalRecord) ([]byte, error)
}

func WithPublicCatalogExporter(exporter PublicCatalogExporter) Option {
	return func(s *Service) { s.catalogExporter = exporter }
}

var publicCatalogID = regexp.MustCompile(`^catalog_[a-f0-9]{48}$`)

type PublicCatalogStore interface {
	PreviewPublicCatalog(context.Context, string, commercial.CatalogRequest) (commercial.CatalogPreview, error)
	ApprovePublicCatalog(context.Context, string, commercial.ApproveCatalogInput, string, time.Time) (commercial.CatalogApprovalRecord, error)
	GetCatalogApproval(context.Context, string) (commercial.CatalogApprovalRecord, error)
	ListCatalogApprovals(context.Context, int) ([]commercial.CatalogApprovalRecord, error)
	MarkCatalogExported(context.Context, string, string, string, time.Time) (commercial.CatalogApprovalRecord, error)
}

func (s *Service) PreviewPublicCatalog(ctx context.Context, input commercial.CatalogRequest, actor string) (commercial.CatalogPreview, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCatalogRead); err != nil {
		return commercial.CatalogPreview{}, err
	}
	if err := input.Validate(); err != nil {
		return commercial.CatalogPreview{}, mapCommercialError(err)
	}
	if !commercialID.MatchString(actor) {
		return commercial.CatalogPreview{}, ErrValidation
	}
	store, ok := s.store.(PublicCatalogStore)
	if !ok {
		return commercial.CatalogPreview{}, ErrBusinessStoreUnavailable
	}
	preview, err := store.PreviewPublicCatalog(ctx, commercialObjectID("catalog", actor, input.OperationID), input)
	return preview, mapCommercialError(err)
}
func (s *Service) ApprovePublicCatalog(ctx context.Context, input commercial.ApproveCatalogInput, actor string) (commercial.CatalogApprovalRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCatalogApprove); err != nil {
		return commercial.CatalogApprovalRecord{}, err
	}
	if err := input.Validate(); err != nil {
		return commercial.CatalogApprovalRecord{}, mapCommercialError(err)
	}
	if !commercialID.MatchString(actor) {
		return commercial.CatalogApprovalRecord{}, ErrValidation
	}
	store, ok := s.store.(PublicCatalogStore)
	if !ok {
		return commercial.CatalogApprovalRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.ApprovePublicCatalog(ctx, commercialObjectID("catalog", actor, input.Request.OperationID), input, actor, s.now().UTC().Truncate(time.Millisecond))
	return record, mapCommercialError(err)
}
func (s *Service) GetCatalogApproval(ctx context.Context, id, actor string) (commercial.CatalogApprovalRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCatalogRead); err != nil {
		return commercial.CatalogApprovalRecord{}, err
	}
	if !publicCatalogID.MatchString(id) {
		return commercial.CatalogApprovalRecord{}, fmt.Errorf("%w: invalid catalog revision", ErrValidation)
	}
	store, ok := s.store.(PublicCatalogStore)
	if !ok {
		return commercial.CatalogApprovalRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetCatalogApproval(ctx, id)
	return record, mapCommercialError(err)
}
func (s *Service) ListCatalogApprovals(ctx context.Context, limit int, actor string) ([]commercial.CatalogApprovalRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCatalogRead); err != nil {
		return nil, err
	}
	store, ok := s.store.(PublicCatalogStore)
	if !ok {
		return nil, ErrBusinessStoreUnavailable
	}
	return store.ListCatalogApprovals(ctx, boundedCommercialLimit(limit))
}

func (s *Service) ExportPublicCatalog(ctx context.Context, id, actor string) (commercial.CatalogApprovalRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCatalogExport); err != nil {
		return commercial.CatalogApprovalRecord{}, err
	}
	if !publicCatalogID.MatchString(id) || !commercialID.MatchString(actor) {
		return commercial.CatalogApprovalRecord{}, ErrValidation
	}
	store, ok := s.store.(PublicCatalogStore)
	if !ok {
		return commercial.CatalogApprovalRecord{}, ErrBusinessStoreUnavailable
	}
	if s.catalogExporter == nil {
		return commercial.CatalogApprovalRecord{}, ErrCatalogExporterUnavailable
	}
	record, err := store.GetCatalogApproval(ctx, id)
	if err != nil {
		return commercial.CatalogApprovalRecord{}, mapCommercialError(err)
	}
	// Always verify the real export, including retries after file/database or
	// response failures. "exported" is a prior event, never proof files still exist.
	if err := s.catalogExporter.Export(ctx, record); err != nil {
		return commercial.CatalogApprovalRecord{}, err
	}
	record, err = store.MarkCatalogExported(ctx, id, record.Public.SHA256, actor, s.now().UTC().Truncate(time.Millisecond))
	return record, mapCommercialError(err)
}
func (s *Service) DownloadPublicCatalog(ctx context.Context, id, actor string) (commercial.CatalogApprovalRecord, []byte, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCatalogRead); err != nil {
		return commercial.CatalogApprovalRecord{}, nil, err
	}
	if !publicCatalogID.MatchString(id) {
		return commercial.CatalogApprovalRecord{}, nil, ErrValidation
	}
	store, ok := s.store.(PublicCatalogStore)
	if !ok {
		return commercial.CatalogApprovalRecord{}, nil, ErrBusinessStoreUnavailable
	}
	if s.catalogExporter == nil {
		return commercial.CatalogApprovalRecord{}, nil, ErrCatalogExporterUnavailable
	}
	record, err := store.GetCatalogApproval(ctx, id)
	if err != nil {
		return commercial.CatalogApprovalRecord{}, nil, mapCommercialError(err)
	}
	if record.Status != "exported" {
		return commercial.CatalogApprovalRecord{}, nil, ErrCatalogNotExported
	}
	raw, err := s.catalogExporter.Read(ctx, record)
	return record, raw, err
}
