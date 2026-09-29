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
	"aster.local/team/operations/backend/internal/ports"
)

const paidFulfillmentSelect = `SELECT id,operation_id,order_id,payment_id,payment_sha256,customer_id,request_id,request_sha256,environment,snapshot_json,content_sha256,approved_by,approved_at,status,claims_json,document_json,document_sha256 FROM commercial_paid_fulfillments`

func paidIntegrity(err error) error {
	return fmt.Errorf("%w: %v", commercial.ErrPaidFulfillmentIntegrity, err)
}

func scanPaidFulfillment(row commercialScanner) (commercial.PaidFulfillmentRecord, error) {
	var r commercial.PaidFulfillmentRecord
	var id, operation, order, payment, paymentHash, customer, request, requestHash, environment, actor string
	var approved time.Time
	var raw, claims, document []byte
	var documentHash sql.NullString
	if err := row.Scan(&id, &operation, &order, &payment, &paymentHash, &customer, &request, &requestHash, &environment, &raw, &r.SHA256, &actor, &approved, &r.Status, &claims, &document, &documentHash); err != nil {
		return r, err
	}
	if err := json.Unmarshal(raw, &r.Snapshot); err != nil {
		return r, paidIntegrity(err)
	}
	v := r.Snapshot
	expected, err := v.Bytes()
	if err != nil || !bytes.Equal(raw, expected) || v.ID != id || v.Request.OperationID != operation || v.Payment.Snapshot.Order.OrderID != order || v.Payment.Snapshot.ID != payment || v.Payment.SHA256 != paymentHash || v.Payment.Snapshot.Order.CustomerID != customer || v.InstallationRequest.RequestID != request || v.RequestSHA256 != requestHash || v.Environment != environment || v.ApprovedBy != actor || v.ApprovedAt != approved.UTC().Format("2006-01-02T15:04:05.000Z") {
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

func validatePaidSources(ctx context.Context, tx *sql.Tx, r commercial.PaidFulfillmentRecord) error {
	payment, err := scanPayment(tx.QueryRowContext(ctx, paymentSelect+" WHERE id=?", r.Snapshot.Payment.Snapshot.ID))
	if err != nil {
		return paidIntegrity(err)
	}
	if err := r.ValidatePaymentSource(payment); err != nil {
		return err
	}
	if err := validatePaymentOrder(ctx, tx, payment); err != nil {
		return paidIntegrity(err)
	}
	return validatePaidLifecycleSource(ctx, tx, r)
}

// Only internal fixed predicates reach this helper. All identifiers are bound.
func (s *Store) getPaidFulfillment(ctx context.Context, predicate, value string) (commercial.PaidFulfillmentRecord, error) {
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable, ReadOnly: true})
	if err != nil {
		return commercial.PaidFulfillmentRecord{}, err
	}
	defer tx.Rollback()
	r, err := scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+predicate, value))
	if errors.Is(err, sql.ErrNoRows) {
		return r, commercial.ErrNotFound
	}
	if err != nil {
		return r, err
	}
	if err := validatePaidSources(ctx, tx, r); err != nil {
		return r, err
	}
	return r, tx.Commit()
}
func (s *Store) GetPaidFulfillment(ctx context.Context, id string) (commercial.PaidFulfillmentRecord, error) {
	return s.getPaidFulfillment(ctx, " WHERE id=?", id)
}
func (s *Store) GetPaidFulfillmentForOrder(ctx context.Context, orderID string) (commercial.PaidFulfillmentRecord, error) {
	return s.getPaidFulfillment(ctx, " WHERE order_id=?", orderID)
}

// previewPaidFulfillment uses an MVCC snapshot for immutable deadline discovery.
// The subsequent write transaction still locks and validates every source.
func (s *Store) previewPaidFulfillment(ctx context.Context, id string) (commercial.PaidFulfillmentRecord, error) {
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted, ReadOnly: true})
	if err != nil {
		return commercial.PaidFulfillmentRecord{}, err
	}
	defer tx.Rollback()
	record, err := scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+" WHERE id=?", id))
	if errors.Is(err, sql.ErrNoRows) {
		return record, commercial.ErrNotFound
	}
	if err != nil {
		return record, err
	}
	if err := validatePaidSources(ctx, tx, record); err != nil {
		return record, err
	}
	if err := tx.Commit(); err != nil {
		return record, err
	}
	return record, nil
}

func matchesPaidApproval(r commercial.PaidFulfillmentRecord, id, orderID, actor string, input []byte) bool {
	stored, err := r.Snapshot.Request.Bytes()
	return err == nil && r.Snapshot.ID == id && r.Snapshot.Payment.Snapshot.Order.OrderID == orderID && r.Snapshot.ApprovedBy == actor && bytes.Equal(stored, input)
}

func claimCommercialInstallationRequest(ctx context.Context, tx *sql.Tx, requestID, requestSHA256, customerID, usageKind string, now time.Time) error {
	if usageKind != "initial" && usageKind != "transfer" && usageKind != "successor" {
		return commercial.ErrConflict
	}
	if _, err := tx.ExecContext(ctx, `INSERT IGNORE INTO commercial_installation_requests(request_id,request_sha256,customer_id,usage_kind,created_at) VALUES(?,?,?,?,?)`, requestID, requestSHA256, customerID, usageKind, now); err != nil {
		return err
	}
	var storedSHA256, storedCustomer, storedUsage string
	if err := tx.QueryRowContext(ctx, "SELECT request_sha256,customer_id,usage_kind FROM commercial_installation_requests WHERE request_id=? FOR UPDATE", requestID).Scan(&storedSHA256, &storedCustomer, &storedUsage); err != nil {
		return err
	}
	if storedSHA256 != requestSHA256 || storedCustomer != customerID || storedUsage != usageKind {
		return commercial.ErrConflict
	}
	return nil
}

func (s *Store) ApprovePaidFulfillment(ctx context.Context, id, orderID string, in commercial.ApprovePaidFulfillmentInput, environment, actor string, refs ports.CustomerReferenceSource) (commercial.PaidFulfillmentRecord, error) {
	requested, err := in.Bytes()
	if err != nil {
		return commercial.PaidFulfillmentRecord{}, err
	}
	old, err := s.getPaidFulfillment(ctx, " WHERE operation_id=?", in.OperationID)
	if err == nil {
		if !matchesPaidApproval(old, id, orderID, actor, requested) {
			return commercial.PaidFulfillmentRecord{}, commercial.ErrConflict
		}
		return old, nil
	}
	if !errors.Is(err, commercial.ErrNotFound) {
		return old, err
	}
	// Only brand-new work depends on current environment and reference capability.
	if environment != "local" && environment != "production" {
		return old, commercial.ErrFulfillmentEnvironment
	}
	preflight, err := s.GetCommercialOrder(ctx, orderID)
	if err != nil {
		return old, err
	}
	end, err := time.Parse(time.RFC3339Nano, preflight.Snapshot.EndsAt)
	if err != nil {
		return old, paidIntegrity(err)
	}
	deadline := end
	// Upgrade and trial-conversion approval must finish while the predecessor is
	// still valid. Resolve that immutable deadline before BeginTx so it covers
	// source-row lock waits, transaction retries and Commit itself.
	if in.Lifecycle != nil && in.Lifecycle.Kind == commercial.PaidLifecycleUpgrade {
		source, sourceErr := s.previewPaidLifecycleSource(ctx, in.Lifecycle, orderID, environment)
		if sourceErr != nil {
			return old, sourceErr
		}
		sourceEnd, parseErr := time.Parse(time.RFC3339Nano, source.ValidUntil)
		if parseErr != nil {
			return old, paidIntegrity(parseErr)
		}
		if sourceEnd.Before(deadline) {
			deadline = sourceEnd
		}
	}
	if !time.Now().Before(deadline) {
		return old, commercial.ErrInvalidPaidFulfillment
	}
	ctx, cancel := context.WithDeadline(ctx, deadline)
	defer cancel()
	var result commercial.PaidFulfillmentRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		old, err := scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+" WHERE operation_id=? FOR UPDATE", in.OperationID))
		if err == nil {
			if !matchesPaidApproval(old, id, orderID, actor, requested) {
				return commercial.ErrConflict
			}
			if err := validatePaidSources(ctx, tx, old); err != nil {
				return err
			}
			result = old
			return tx.Commit()
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		order, err := scanCommercialOrder(tx.QueryRowContext(ctx, commercialOrderSelect+" WHERE id=? FOR UPDATE", orderID))
		if err != nil {
			return err
		}
		if order.SHA256 != in.ExpectedOrderSHA256 || order.SHA256 != preflight.SHA256 || order.Status != "fulfillment_pending" {
			return commercial.ErrConflict
		}
		if order.Snapshot.Source != nil && order.Snapshot.Source.Environment != environment {
			return commercial.ErrFulfillmentEnvironment
		}
		var existing string
		err = tx.QueryRowContext(ctx, "SELECT id FROM commercial_paid_fulfillments WHERE order_id=? FOR UPDATE", orderID).Scan(&existing)
		if err == nil {
			return commercial.ErrConflict
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		payment, err := scanPayment(tx.QueryRowContext(ctx, paymentSelect+" WHERE order_id=? FOR UPDATE", orderID))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrNotFound
		}
		if err != nil {
			return err
		}
		if payment.SHA256 != in.ExpectedPaymentSHA256 {
			return commercial.ErrConflict
		}
		if err := validatePaymentOrder(ctx, tx, payment); err != nil {
			return paidIntegrity(err)
		}
		var customer string
		if err := tx.QueryRowContext(ctx, "SELECT id FROM customers WHERE id=? AND status<>'inactive' FOR UPDATE", order.Snapshot.CustomerID).Scan(&customer); err != nil {
			if errors.Is(err, sql.ErrNoRows) {
				return commercial.ErrConflict
			}
			return err
		}
		// Legacy customer IDs use a case-insensitive database collation. Never
		// bind an immutable order under an alias or commit an unreadable record.
		if customer != order.Snapshot.CustomerID {
			return commercial.ErrConflict
		}
		if refs == nil {
			return commercial.ErrCustomerReferenceUnavailable
		}
		customerRef, err := refs.Reference(customer)
		if err != nil {
			return fmt.Errorf("%w: %v", commercial.ErrCustomerReferenceUnavailable, err)
		}
		lifecycle, err := resolvePaidLifecycleSource(ctx, tx, in.Lifecycle, orderID, environment, true)
		if err != nil {
			return err
		}
		// The source resolver above may wait for predecessor locks. Use a fresh
		// timestamp after those locks are held, never the pre-wait observation.
		now := time.Now().UTC().Truncate(time.Millisecond)
		result, err = commercial.NewPaidFulfillmentWithLifecycle(id, in, payment, customerRef, environment, actor, now, lifecycle)
		if err != nil {
			return err
		}
		raw, err := result.Snapshot.Bytes()
		if err != nil {
			return err
		}
		v := result.Snapshot
		// A request ID denotes one complete machine request for one customer,
		// including when reused by another order. Lock the indexed range so
		// concurrent first uses cannot introduce different bindings under one ID.
		rows, err := tx.QueryContext(ctx, paidFulfillmentSelect+" WHERE request_id=? FOR UPDATE", v.InstallationRequest.RequestID)
		if err != nil {
			return err
		}
		for rows.Next() {
			previous, err := scanPaidFulfillment(rows)
			if err != nil {
				rows.Close()
				return err
			}
			before, err := json.Marshal(previous.Snapshot.InstallationRequest)
			if err != nil {
				rows.Close()
				return err
			}
			after, err := json.Marshal(v.InstallationRequest)
			if err != nil || previous.Snapshot.Payment.Snapshot.Order.CustomerID != customer || !bytes.Equal(before, after) {
				rows.Close()
				return commercial.ErrConflict
			}
		}
		if err := rows.Err(); err != nil {
			rows.Close()
			return err
		}
		if err := rows.Close(); err != nil {
			return err
		}
		usageKind := "initial"
		if lifecycle != nil {
			usageKind = "successor"
		}
		if err := claimCommercialInstallationRequest(ctx, tx, v.InstallationRequest.RequestID, v.RequestSHA256, customer, usageKind, now); err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO commercial_paid_fulfillments(id,operation_id,order_id,payment_id,payment_sha256,customer_id,request_id,request_sha256,environment,snapshot_json,content_sha256,approved_by,approved_at,status) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,'approved')`, id, in.OperationID, orderID, payment.Snapshot.ID, payment.SHA256, customer, v.InstallationRequest.RequestID, v.RequestSHA256, environment, raw, result.SHA256, actor, now); err != nil {
			return commercialConflict(err)
		}
		if err := storePaidLifecycleSource(ctx, tx, id, lifecycle, now); err != nil {
			return err
		}
		if err := writeCommercialAudit(ctx, tx, in.OperationID, "commercial.fulfillment_approved", id, actor, result.SHA256, now); err != nil {
			return err
		}
		if !time.Now().Before(deadline) {
			return commercial.ErrInvalidPaidFulfillment
		}
		return tx.Commit()
	})
	return result, err
}

func validatePaidProgress(ctx context.Context, tx *sql.Tx, r commercial.PaidFulfillmentRecord, environment string) error {
	if environment != r.Snapshot.Environment {
		return commercial.ErrFulfillmentEnvironment
	}
	if err := validatePaidSources(ctx, tx, r); err != nil {
		return err
	}
	var status string
	if err := tx.QueryRowContext(ctx, "SELECT status FROM commercial_orders WHERE id=? FOR UPDATE", r.Snapshot.Payment.Snapshot.Order.OrderID).Scan(&status); err != nil {
		return err
	}
	if status != "fulfillment_pending" {
		return commercial.ErrConflict
	}
	if err := tx.QueryRowContext(ctx, "SELECT status FROM customers WHERE id=? FOR UPDATE", r.Snapshot.Payment.Snapshot.Order.CustomerID).Scan(&status); err != nil {
		return err
	}
	if status == "inactive" {
		return commercial.ErrConflict
	}
	return nil
}

func (s *Store) PreparePaidFulfillment(ctx context.Context, id, keyID, environment, actor string) (commercial.PaidFulfillmentRecord, error) {
	current, err := s.GetPaidFulfillment(ctx, id)
	if err != nil {
		return current, err
	}
	if current.Claims != nil && current.Claims.KeyID != keyID {
		return current, commercial.ErrConflict
	}
	if current.Status == "issued" {
		return current, nil
	}
	// Only first preparation is bounded by the original expiry. Recovery of
	// already prepared claims finishes the original transaction after expiry.
	if current.Status == "approved" {
		end, _ := time.Parse(time.RFC3339Nano, current.Snapshot.Payment.Snapshot.Order.EndsAt)
		if !time.Now().Before(end) {
			return current, commercial.ErrInvalidPaidFulfillment
		}
		var cancel context.CancelFunc
		ctx, cancel = context.WithDeadline(ctx, end)
		defer cancel()
	}
	var result commercial.PaidFulfillmentRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		result, err = scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+" WHERE id=? FOR UPDATE", id))
		if err != nil {
			return err
		}
		if result.Claims != nil && result.Claims.KeyID != keyID {
			return commercial.ErrConflict
		}
		if result.Status == "issued" {
			if err := validatePaidSources(ctx, tx, result); err != nil {
				return err
			}
			return tx.Commit()
		}
		if err := validatePaidProgress(ctx, tx, result, environment); err != nil {
			return err
		}
		if result.Status == "prepared" {
			return tx.Commit()
		}
		now := time.Now().UTC().Truncate(time.Millisecond)
		claims, err := result.Snapshot.Claims(keyID, now)
		if err != nil {
			return fmt.Errorf("%w: %v", commercial.ErrInvalidPaidFulfillment, err)
		}
		result.Status, result.Claims = "prepared", &claims
		if err := result.Validate(); err != nil {
			return err
		}
		raw, err := json.Marshal(claims)
		if err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx, "UPDATE commercial_paid_fulfillments SET status='prepared',claims_json=? WHERE id=? AND status='approved'", raw, id); err != nil {
			return err
		}
		if err := writeCommercialAudit(ctx, tx, id, "commercial.fulfillment_prepared", id, actor, commercial.ContentDigest(raw), now); err != nil {
			return err
		}
		if err := ctx.Err(); err != nil {
			return err
		}
		return tx.Commit()
	})
	return result, err
}

func (s *Store) CompletePaidFulfillment(ctx context.Context, id string, document licenseprotocol.DocumentV2, environment, actor string) (commercial.PaidFulfillmentRecord, error) {
	raw, err := json.Marshal(document)
	if err != nil {
		return commercial.PaidFulfillmentRecord{}, err
	}
	digest := commercial.ContentDigest(raw)
	var result commercial.PaidFulfillmentRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		result, err = scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+" WHERE id=? FOR UPDATE", id))
		if err != nil {
			return err
		}
		if err := validatePaidSources(ctx, tx, result); err != nil {
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
		if err := validatePaidProgress(ctx, tx, result, environment); err != nil {
			return err
		}
		result.Status, result.Document, result.DocumentSHA256 = "issued", &document, digest
		if err := result.Validate(); err != nil {
			return err
		}
		now := time.Now().UTC().Truncate(time.Millisecond)
		if _, err := tx.ExecContext(ctx, "UPDATE commercial_paid_fulfillments SET status='issued',document_json=?,document_sha256=? WHERE id=? AND status='prepared'", raw, digest, id); err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx, "UPDATE commercial_orders SET status='fulfilled',updated_at=? WHERE id=? AND status='fulfillment_pending'", now, result.Snapshot.Payment.Snapshot.Order.OrderID); err != nil {
			return err
		}
		if err := writeCommercialAudit(ctx, tx, id, "commercial.fulfillment_issued", id, actor, digest, now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return result, err
}
