package mariadb

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/licenseprotocol"
)

const paidTransferSelect = `SELECT id,operation_id,fulfillment_id,fulfillment_sha256,customer_id,request_id,request_sha256,previous_document_sha256,transfer_sequence,environment,snapshot_json,content_sha256,approved_by,approved_at,status,claims_json,document_json,document_sha256 FROM commercial_paid_transfers`

func scanPaidTransfer(row commercialScanner) (commercial.PaidTransferRecord, error) {
	var r commercial.PaidTransferRecord
	var id, operation, fulfillment, fulfillmentHash, customer, request, requestHash, previousHash, environment, actor string
	var sequence uint32
	var approved time.Time
	var snapshot, claims, document []byte
	var documentHash sql.NullString
	if err := row.Scan(&id, &operation, &fulfillment, &fulfillmentHash, &customer, &request, &requestHash, &previousHash, &sequence, &environment, &snapshot, &r.SHA256, &actor, &approved, &r.Status, &claims, &document, &documentHash); err != nil {
		return r, err
	}
	if err := json.Unmarshal(snapshot, &r.Snapshot); err != nil {
		return r, paidIntegrity(err)
	}
	v := r.Snapshot
	expected, err := v.Bytes()
	if err != nil || !bytes.Equal(snapshot, expected) || v.ID != id || v.Request.OperationID != operation ||
		v.FulfillmentID != fulfillment || v.FulfillmentSHA256 != fulfillmentHash || v.CustomerID != customer ||
		v.InstallationRequest.RequestID != request || v.RequestSHA256 != requestHash || v.PreviousDocumentSHA256 != previousHash ||
		v.TransferSequence != sequence || v.Environment != environment || v.ApprovedBy != actor ||
		v.ApprovedAt != approved.UTC().Format("2006-01-02T15:04:05.000Z") {
		return r, commercial.ErrPaidFulfillmentIntegrity
	}
	if len(claims) > 0 {
		r.Claims = &licenseprotocol.ClaimsV2{}
		if err := json.Unmarshal(claims, r.Claims); err != nil {
			return r, paidIntegrity(err)
		}
		expected, err := json.Marshal(r.Claims)
		if err != nil || !bytes.Equal(claims, expected) {
			return r, commercial.ErrPaidFulfillmentIntegrity
		}
	}
	if len(document) > 0 {
		r.Document = &licenseprotocol.DocumentV2{}
		if err := json.Unmarshal(document, r.Document); err != nil {
			return r, paidIntegrity(err)
		}
		expected, err := json.Marshal(r.Document)
		if err != nil || !bytes.Equal(document, expected) {
			return r, commercial.ErrPaidFulfillmentIntegrity
		}
	}
	r.DocumentSHA256 = documentHash.String
	return r, r.Validate()
}

func validatePaidTransferSources(ctx context.Context, tx *sql.Tx, record commercial.PaidTransferRecord) error {
	fulfillment, err := scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+" WHERE id=?", record.Snapshot.FulfillmentID))
	if err != nil {
		return paidIntegrity(err)
	}
	if err := validatePaidSources(ctx, tx, fulfillment); err != nil {
		return err
	}
	if err := record.ValidateFulfillment(fulfillment); err != nil {
		return err
	}
	current := record
	for {
		if current.Snapshot.TransferSequence == 1 {
			if fulfillment.Claims == nil || current.ValidatePredecessor(*fulfillment.Claims, fulfillment.DocumentSHA256) != nil {
				return commercial.ErrPaidFulfillmentIntegrity
			}
			return nil
		}
		previous, err := scanPaidTransfer(tx.QueryRowContext(ctx, paidTransferSelect+" WHERE fulfillment_id=? AND transfer_sequence=?", current.Snapshot.FulfillmentID, current.Snapshot.TransferSequence-1))
		if err != nil || previous.Status != "issued" || previous.Claims == nil || previous.Document == nil ||
			current.ValidatePredecessor(*previous.Claims, previous.DocumentSHA256) != nil {
			return paidIntegrity(err)
		}
		if err := previous.ValidateFulfillment(fulfillment); err != nil {
			return err
		}
		current = previous
	}
}

func (s *Store) getPaidTransfer(ctx context.Context, query string, args ...any) (commercial.PaidTransferRecord, error) {
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable, ReadOnly: true})
	if err != nil {
		return commercial.PaidTransferRecord{}, err
	}
	defer tx.Rollback()
	record, err := scanPaidTransfer(tx.QueryRowContext(ctx, query, args...))
	if errors.Is(err, sql.ErrNoRows) {
		return record, commercial.ErrNotFound
	}
	if err != nil {
		return record, err
	}
	if err := validatePaidTransferSources(ctx, tx, record); err != nil {
		return record, err
	}
	return record, tx.Commit()
}

func (s *Store) GetPaidTransfer(ctx context.Context, id string) (commercial.PaidTransferRecord, error) {
	return s.getPaidTransfer(ctx, paidTransferSelect+" WHERE id=?", id)
}

func (s *Store) GetLatestPaidTransfer(ctx context.Context, fulfillmentID string) (commercial.PaidTransferRecord, error) {
	return s.getPaidTransfer(ctx, paidTransferSelect+" WHERE fulfillment_id=? ORDER BY transfer_sequence DESC LIMIT 1", fulfillmentID)
}

func (s *Store) ListPaidTransferChain(ctx context.Context, fulfillmentID string, throughSequence uint32) ([]commercial.PaidTransferRecord, error) {
	if throughSequence == 0 || throughSequence > 10000 {
		return nil, commercial.ErrPaidFulfillmentIntegrity
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable, ReadOnly: true})
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()
	fulfillment, err := scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+" WHERE id=?", fulfillmentID))
	if err != nil {
		return nil, paidIntegrity(err)
	}
	if err := validatePaidSources(ctx, tx, fulfillment); err != nil {
		return nil, err
	}
	rows, err := tx.QueryContext(ctx, paidTransferSelect+" WHERE fulfillment_id=? AND transfer_sequence<=? ORDER BY transfer_sequence", fulfillmentID, throughSequence)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := make([]commercial.PaidTransferRecord, 0, throughSequence)
	if fulfillment.Claims == nil {
		return nil, commercial.ErrPaidFulfillmentIntegrity
	}
	previousClaims, previousDocumentSHA256 := *fulfillment.Claims, fulfillment.DocumentSHA256
	for rows.Next() {
		record, err := scanPaidTransfer(rows)
		if err != nil || record.Snapshot.TransferSequence != uint32(len(result)+1) || record.ValidateFulfillment(fulfillment) != nil || record.ValidatePredecessor(previousClaims, previousDocumentSHA256) != nil {
			return nil, paidIntegrity(err)
		}
		if record.Snapshot.TransferSequence < throughSequence && (record.Status != "issued" || record.Claims == nil || record.Document == nil) {
			return nil, commercial.ErrPaidFulfillmentIntegrity
		}
		if record.Claims != nil {
			previousClaims, previousDocumentSHA256 = *record.Claims, record.DocumentSHA256
		}
		result = append(result, record)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	if len(result) != int(throughSequence) {
		return nil, commercial.ErrPaidFulfillmentIntegrity
	}
	return result, tx.Commit()
}

func (s *Store) ApprovePaidTransfer(ctx context.Context, id, fulfillmentID string, input commercial.ApprovePaidTransferInput, environment, actor string) (commercial.PaidTransferRecord, error) {
	requested, err := input.Bytes()
	if err != nil {
		return commercial.PaidTransferRecord{}, err
	}
	old, err := s.getPaidTransfer(ctx, paidTransferSelect+" WHERE operation_id=?", input.OperationID)
	if err == nil {
		if !old.Matches(id, fulfillmentID, actor, requested) {
			return old, commercial.ErrConflict
		}
		return old, nil
	}
	if !errors.Is(err, commercial.ErrNotFound) {
		return old, err
	}
	if environment != "local" && environment != "production" {
		return old, commercial.ErrFulfillmentEnvironment
	}
	installationRequest, err := licenseprotocol.ParseRequestV2([]byte(input.LicenseRequestJSON))
	if err != nil {
		return old, commercial.ErrInvalidPaidTransfer
	}
	preflight, err := s.previewPaidFulfillment(ctx, fulfillmentID)
	if err != nil {
		return old, err
	}
	if preflight.Status != "issued" || preflight.Claims == nil || preflight.Document == nil {
		return old, commercial.ErrConflict
	}
	deadline, err := time.Parse(time.RFC3339Nano, preflight.Claims.Validity.Expiry.ExpiresAt)
	if err != nil {
		return old, paidIntegrity(err)
	}
	if !time.Now().Before(deadline) {
		return old, commercial.ErrInvalidPaidTransfer
	}
	// Bind the transaction lifetime to the original license expiry so request
	// registry locks, audit writes and Commit cannot produce a transfer after
	// the predecessor has expired.
	ctx, cancel := context.WithDeadline(ctx, deadline)
	defer cancel()
	var result commercial.PaidTransferRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		old, err := scanPaidTransfer(tx.QueryRowContext(ctx, paidTransferSelect+" WHERE operation_id=? FOR UPDATE", input.OperationID))
		if err == nil {
			if !old.Matches(id, fulfillmentID, actor, requested) {
				return commercial.ErrConflict
			}
			if err := validatePaidTransferSources(ctx, tx, old); err != nil {
				return err
			}
			result = old
			return tx.Commit()
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		fulfillment, err := scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+" WHERE id=? FOR UPDATE", fulfillmentID))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrNotFound
		}
		if err != nil {
			return err
		}
		if err := validatePaidSources(ctx, tx, fulfillment); err != nil {
			return err
		}
		if fulfillment.Status != "issued" || fulfillment.Claims == nil || fulfillment.Document == nil {
			return commercial.ErrConflict
		}
		if fulfillment.Snapshot.Environment != environment {
			return commercial.ErrFulfillmentEnvironment
		}
		previousClaims, previousHash := *fulfillment.Claims, fulfillment.DocumentSHA256
		latest, err := scanPaidTransfer(tx.QueryRowContext(ctx, paidTransferSelect+" WHERE fulfillment_id=? ORDER BY transfer_sequence DESC LIMIT 1 FOR UPDATE", fulfillmentID))
		if err == nil {
			if err := validatePaidTransferSources(ctx, tx, latest); err != nil {
				return err
			}
			if latest.Status != "issued" || latest.Claims == nil || latest.Document == nil {
				return commercial.ErrConflict
			}
			previousClaims, previousHash = *latest.Claims, latest.DocumentSHA256
		} else if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		var successor string
		if err := tx.QueryRowContext(ctx, "SELECT fulfillment_id FROM commercial_paid_lifecycle_sources WHERE source_namespace='paid' AND source_id=? FOR UPDATE", fulfillmentID).Scan(&successor); err == nil {
			return commercial.ErrConflict
		} else if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if input.ExpectedCurrentDocumentSHA256 != previousHash {
			return commercial.ErrConflict
		}
		var existing string
		if err := tx.QueryRowContext(ctx, "SELECT id FROM commercial_paid_fulfillments WHERE request_id=? FOR UPDATE", installationRequest.RequestID).Scan(&existing); err == nil {
			return commercial.ErrConflict
		} else if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if err := tx.QueryRowContext(ctx, "SELECT id FROM commercial_paid_transfers WHERE request_id=? FOR UPDATE", installationRequest.RequestID).Scan(&existing); err == nil {
			return commercial.ErrConflict
		} else if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		now := time.Now().UTC().Truncate(time.Millisecond)
		result, err = commercial.NewPaidTransfer(id, input, fulfillment, previousClaims, previousHash, actor, now)
		if err != nil {
			return err
		}
		raw, err := result.Snapshot.Bytes()
		if err != nil {
			return err
		}
		v := result.Snapshot
		if err := claimCommercialInstallationRequest(ctx, tx, v.InstallationRequest.RequestID, v.RequestSHA256, v.CustomerID, "transfer", now); err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO commercial_paid_transfers(id,operation_id,fulfillment_id,fulfillment_sha256,customer_id,request_id,request_sha256,previous_document_sha256,transfer_sequence,environment,snapshot_json,content_sha256,approved_by,approved_at,status) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,'approved')`, v.ID, input.OperationID, v.FulfillmentID, v.FulfillmentSHA256, v.CustomerID, v.InstallationRequest.RequestID, v.RequestSHA256, v.PreviousDocumentSHA256, v.TransferSequence, v.Environment, raw, result.SHA256, v.ApprovedBy, now); err != nil {
			return commercialConflict(err)
		}
		if err := writeCommercialAudit(ctx, tx, input.OperationID, "commercial.fulfillment_transfer_approved", id, actor, result.SHA256, now); err != nil {
			return err
		}
		if !time.Now().Before(deadline) {
			return commercial.ErrInvalidPaidTransfer
		}
		return tx.Commit()
	})
	return result, err
}

func validatePaidTransferProgress(ctx context.Context, tx *sql.Tx, record commercial.PaidTransferRecord, environment string) error {
	if record.Snapshot.Environment != environment {
		return commercial.ErrFulfillmentEnvironment
	}
	if err := validatePaidTransferSources(ctx, tx, record); err != nil {
		return err
	}
	var latestID string
	if err := tx.QueryRowContext(ctx, "SELECT id FROM commercial_paid_transfers WHERE fulfillment_id=? ORDER BY transfer_sequence DESC LIMIT 1 FOR UPDATE", record.Snapshot.FulfillmentID).Scan(&latestID); err != nil {
		return err
	}
	if latestID != record.Snapshot.ID {
		return commercial.ErrConflict
	}
	return nil
}

func (s *Store) PreparePaidTransfer(ctx context.Context, id, keyID, environment, actor string) (commercial.PaidTransferRecord, error) {
	current, err := s.GetPaidTransfer(ctx, id)
	if err != nil {
		return current, err
	}
	if current.Claims != nil && current.Claims.KeyID != keyID {
		return current, commercial.ErrConflict
	}
	if current.Status == "issued" {
		return current, nil
	}
	if current.Status == "approved" {
		end, _ := time.Parse(time.RFC3339Nano, current.Snapshot.OriginalClaims.Validity.Expiry.ExpiresAt)
		if !time.Now().Before(end) {
			return current, commercial.ErrInvalidPaidTransfer
		}
		var cancel context.CancelFunc
		ctx, cancel = context.WithDeadline(ctx, end)
		defer cancel()
	}
	var result commercial.PaidTransferRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		result, err = scanPaidTransfer(tx.QueryRowContext(ctx, paidTransferSelect+" WHERE id=? FOR UPDATE", id))
		if err != nil {
			return err
		}
		if result.Claims != nil && result.Claims.KeyID != keyID {
			return commercial.ErrConflict
		}
		if result.Status == "issued" {
			if err := validatePaidTransferSources(ctx, tx, result); err != nil {
				return err
			}
			return tx.Commit()
		}
		if err := validatePaidTransferProgress(ctx, tx, result, environment); err != nil {
			return err
		}
		if result.Status == "prepared" {
			return tx.Commit()
		}
		now := time.Now().UTC().Truncate(time.Millisecond)
		claims, err := result.Snapshot.Claims(keyID, now)
		if err != nil {
			return fmt.Errorf("%w: %v", commercial.ErrInvalidPaidTransfer, err)
		}
		result.Status, result.Claims = "prepared", &claims
		if err := result.Validate(); err != nil {
			return err
		}
		raw, err := json.Marshal(claims)
		if err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx, "UPDATE commercial_paid_transfers SET status='prepared',claims_json=? WHERE id=? AND status='approved'", raw, id); err != nil {
			return err
		}
		if err := writeCommercialAudit(ctx, tx, id, "commercial.fulfillment_transfer_prepared", id, actor, commercial.ContentDigest(raw), now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return result, err
}

func (s *Store) CompletePaidTransfer(ctx context.Context, id string, document licenseprotocol.DocumentV2, environment, actor string) (commercial.PaidTransferRecord, error) {
	raw, err := json.Marshal(document)
	if err != nil {
		return commercial.PaidTransferRecord{}, err
	}
	digest := commercial.ContentDigest(raw)
	var result commercial.PaidTransferRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		result, err = scanPaidTransfer(tx.QueryRowContext(ctx, paidTransferSelect+" WHERE id=? FOR UPDATE", id))
		if err != nil {
			return err
		}
		if err := validatePaidTransferSources(ctx, tx, result); err != nil {
			return err
		}
		if result.Status == "issued" {
			if result.DocumentSHA256 != digest {
				return commercial.ErrConflict
			}
			return tx.Commit()
		}
		if result.Status != "prepared" {
			return commercial.ErrConflict
		}
		if err := validatePaidTransferProgress(ctx, tx, result, environment); err != nil {
			return err
		}
		result.Status, result.Document, result.DocumentSHA256 = "issued", &document, digest
		if err := result.Validate(); err != nil {
			return err
		}
		now := time.Now().UTC().Truncate(time.Millisecond)
		if _, err := tx.ExecContext(ctx, "UPDATE commercial_paid_transfers SET status='issued',document_json=?,document_sha256=? WHERE id=? AND status='prepared'", raw, digest, id); err != nil {
			return err
		}
		if err := writeCommercialAudit(ctx, tx, id, "commercial.fulfillment_transfer_issued", id, actor, digest, now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return result, err
}
