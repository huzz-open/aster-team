package mariadb

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/licenseprotocol"
)

const paidLifecycleSelect = `SELECT fulfillment_id,kind,source_id,source_record_kind,source_record_id,source_record_sha256,document_sha256,snapshot_json,content_sha256 FROM commercial_paid_lifecycle_sources`

func currentPaidLifecycleSource(ctx context.Context, tx *sql.Tx, request commercial.PaidLifecycleRequest, environment string, lock bool) (commercial.PaidLifecycleSource, error) {
	if environment != "local" && environment != "production" {
		return commercial.PaidLifecycleSource{}, commercial.ErrFulfillmentEnvironment
	}
	lockSQL := ""
	if lock {
		lockSQL = " FOR UPDATE"
	}
	fulfillment, err := scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+" WHERE id=?"+lockSQL, request.SourceID))
	if errors.Is(err, sql.ErrNoRows) {
		return commercial.PaidLifecycleSource{}, commercial.ErrNotFound
	}
	if err != nil {
		return commercial.PaidLifecycleSource{}, err
	}
	if err := validatePaidSources(ctx, tx, fulfillment); err != nil {
		return commercial.PaidLifecycleSource{}, err
	}
	if fulfillment.Snapshot.Environment != environment {
		return commercial.PaidLifecycleSource{}, commercial.ErrFulfillmentEnvironment
	}
	if fulfillment.Status != "issued" || fulfillment.Claims == nil || fulfillment.Document == nil {
		return commercial.PaidLifecycleSource{}, commercial.ErrInvalidPaidLifecycle
	}

	currentClaims := *fulfillment.Claims
	currentDocument := fulfillment.Document
	currentDocumentSHA256 := fulfillment.DocumentSHA256
	currentRecordKind, currentRecordID, currentRecordSHA256 := "paid_fulfillment", fulfillment.Snapshot.ID, fulfillment.SHA256
	rows, err := tx.QueryContext(ctx, paidTransferSelect+" WHERE fulfillment_id=? ORDER BY transfer_sequence"+lockSQL, fulfillment.Snapshot.ID)
	if err != nil {
		return commercial.PaidLifecycleSource{}, err
	}
	defer rows.Close()
	for rows.Next() {
		transfer, err := scanPaidTransfer(rows)
		if err != nil || transfer.ValidateFulfillment(fulfillment) != nil || transfer.ValidatePredecessor(currentClaims, currentDocumentSHA256) != nil || transfer.Status != "issued" || transfer.Claims == nil || transfer.Document == nil {
			return commercial.PaidLifecycleSource{}, commercial.ErrPaidFulfillmentIntegrity
		}
		currentClaims = *transfer.Claims
		currentDocument = transfer.Document
		currentDocumentSHA256 = transfer.DocumentSHA256
		currentRecordKind, currentRecordID, currentRecordSHA256 = "paid_transfer", transfer.Snapshot.ID, transfer.SHA256
	}
	if err := rows.Err(); err != nil {
		return commercial.PaidLifecycleSource{}, err
	}
	if currentClaims.Binding.Mode != licenseprotocol.InstallationV2 || currentClaims.Binding.TransferSequence == nil || currentClaims.Validity.Expiry.Mode != licenseprotocol.FixedExpiryV2 {
		return commercial.PaidLifecycleSource{}, commercial.ErrPaidFulfillmentIntegrity
	}
	return commercial.PaidLifecycleSource{
		Schema: commercial.PaidLifecycleSourceSchema, Kind: request.Kind, SourceID: fulfillment.Snapshot.ID,
		SourceRecordKind: currentRecordKind, SourceRecordID: currentRecordID, SourceRecordSHA256: currentRecordSHA256,
		CustomerID: fulfillment.Snapshot.Payment.Snapshot.Order.CustomerID, CustomerRef: currentClaims.Source.CustomerRef, LicenseID: currentClaims.LicenseID,
		Environment: environment, DocumentSHA256: currentDocumentSHA256, Document: currentDocument, Binding: currentClaims.Binding,
		ValidFrom: currentClaims.Validity.NotBefore, ValidUntil: currentClaims.Validity.Expiry.ExpiresAt,
	}, nil
}

func resolvePaidLifecycleSource(ctx context.Context, tx *sql.Tx, request *commercial.PaidLifecycleRequest, successorOrderID, environment string, lock bool) (*commercial.PaidLifecycleSource, error) {
	if request == nil {
		return nil, nil
	}
	if err := request.Validate(); err != nil {
		return nil, err
	}
	source, err := currentPaidLifecycleSource(ctx, tx, *request, environment, lock)
	if err != nil {
		return nil, err
	}
	if request.ExpectedDocumentSHA256 != source.DocumentSHA256 {
		return nil, commercial.ErrConflict
	}
	return &source, source.Validate()
}

// previewPaidLifecycleSource reads the predecessor through an MVCC snapshot so
// approval can bind its context deadline before waiting on the authoritative
// FOR UPDATE locks. The write transaction resolves and compares it again.
func (s *Store) previewPaidLifecycleSource(ctx context.Context, request *commercial.PaidLifecycleRequest, successorOrderID, environment string) (*commercial.PaidLifecycleSource, error) {
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted, ReadOnly: true})
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()
	source, err := resolvePaidLifecycleSource(ctx, tx, request, successorOrderID, environment, false)
	if err != nil {
		return nil, err
	}
	if err := tx.Commit(); err != nil {
		return nil, err
	}
	return source, nil
}

func (s *Store) GetPaidLifecycleSource(ctx context.Context, kind, sourceID, environment string) (commercial.PaidLifecycleSource, error) {
	request := commercial.PaidLifecycleRequest{Kind: kind, SourceID: sourceID, ExpectedDocumentSHA256: ""}
	if kind != commercial.PaidLifecycleRenewal && kind != commercial.PaidLifecycleUpgrade {
		return commercial.PaidLifecycleSource{}, commercial.ErrInvalidPaidLifecycle
	}
	if sourceID == "" {
		return commercial.PaidLifecycleSource{}, commercial.ErrInvalidPaidLifecycle
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return commercial.PaidLifecycleSource{}, err
	}
	defer tx.Rollback()
	source, err := currentPaidLifecycleSource(ctx, tx, request, environment, true)
	if err != nil {
		return source, err
	}
	var existing string
	namespace := "paid"
	if err := tx.QueryRowContext(ctx, "SELECT fulfillment_id FROM commercial_paid_lifecycle_sources WHERE source_namespace=? AND source_id=? FOR UPDATE", namespace, sourceID).Scan(&existing); err == nil {
		return source, commercial.ErrConflict
	} else if !errors.Is(err, sql.ErrNoRows) {
		return source, err
	}
	return source, tx.Commit()
}

func storePaidLifecycleSource(ctx context.Context, tx *sql.Tx, fulfillmentID string, source *commercial.PaidLifecycleSource, now time.Time) error {
	if source == nil {
		return nil
	}
	raw, err := source.Bytes()
	if err != nil {
		return err
	}
	namespace := "paid"
	if _, err := tx.ExecContext(ctx, `INSERT INTO commercial_paid_lifecycle_sources(fulfillment_id,source_namespace,kind,source_id,source_record_kind,source_record_id,source_record_sha256,document_sha256,snapshot_json,content_sha256,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)`, fulfillmentID, namespace, source.Kind, source.SourceID, source.SourceRecordKind, source.SourceRecordID, source.SourceRecordSHA256, source.DocumentSHA256, raw, commercial.ContentDigest(raw), now); err != nil {
		return commercialConflict(err)
	}
	return nil
}

func validatePaidLifecycleSource(ctx context.Context, tx *sql.Tx, record commercial.PaidFulfillmentRecord) error {
	if record.Snapshot.Lifecycle == nil {
		return nil
	}
	var fulfillmentID, kind, sourceID, sourceRecordKind, sourceRecordID, sourceRecordSHA256, documentSHA256, contentSHA256 string
	var raw []byte
	if err := tx.QueryRowContext(ctx, paidLifecycleSelect+" WHERE fulfillment_id=?", record.Snapshot.ID).Scan(&fulfillmentID, &kind, &sourceID, &sourceRecordKind, &sourceRecordID, &sourceRecordSHA256, &documentSHA256, &raw, &contentSHA256); err != nil {
		return paidIntegrity(err)
	}
	var stored commercial.PaidLifecycleSource
	if err := json.Unmarshal(raw, &stored); err != nil {
		return paidIntegrity(err)
	}
	expected, err := stored.Bytes()
	if err != nil || !bytes.Equal(raw, expected) || commercial.ContentDigest(raw) != contentSHA256 || fulfillmentID != record.Snapshot.ID || kind != stored.Kind || sourceID != stored.SourceID || sourceRecordKind != stored.SourceRecordKind || sourceRecordID != stored.SourceRecordID || sourceRecordSHA256 != stored.SourceRecordSHA256 || documentSHA256 != stored.DocumentSHA256 {
		return commercial.ErrPaidFulfillmentIntegrity
	}
	want, err := record.Snapshot.Lifecycle.Bytes()
	if err != nil || !bytes.Equal(want, expected) {
		return commercial.ErrPaidFulfillmentIntegrity
	}
	current, err := resolvePaidLifecycleSource(ctx, tx, record.Snapshot.Request.Lifecycle, record.Snapshot.Payment.Snapshot.Order.OrderID, record.Snapshot.Environment, false)
	if err != nil {
		return err
	}
	actual, err := current.Bytes()
	if err != nil || !bytes.Equal(actual, expected) {
		return commercial.ErrConflict
	}
	return nil
}
