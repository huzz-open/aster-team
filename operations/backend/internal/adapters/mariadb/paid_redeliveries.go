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

const paidRedeliverySelect = `SELECT id,operation_id,fulfillment_id,fulfillment_sha256,order_id,customer_id,document_sha256,environment,snapshot_json,content_sha256,requested_by,requested_at FROM commercial_paid_redeliveries`

func scanPaidRedelivery(row commercialScanner) (commercial.PaidRedeliveryRecord, error) {
	var result commercial.PaidRedeliveryRecord
	var id, operationID, fulfillmentID, fulfillmentSHA256, orderID, customerID, documentSHA256, environment, actor string
	var snapshot []byte
	var requestedAt time.Time
	if err := row.Scan(&id, &operationID, &fulfillmentID, &fulfillmentSHA256, &orderID, &customerID, &documentSHA256, &environment, &snapshot, &result.SHA256, &actor, &requestedAt); err != nil {
		return result, err
	}
	if err := json.Unmarshal(snapshot, &result.Snapshot); err != nil {
		return result, paidIntegrity(err)
	}
	expected, err := result.Snapshot.Bytes()
	v := result.Snapshot
	if err != nil || !bytes.Equal(snapshot, expected) || v.ID != id || v.Request.OperationID != operationID ||
		v.FulfillmentID != fulfillmentID || v.FulfillmentSHA256 != fulfillmentSHA256 || v.OrderID != orderID ||
		v.CustomerID != customerID || v.DocumentSHA256 != documentSHA256 || v.Environment != environment ||
		v.RequestedBy != actor || v.RequestedAt != requestedAt.UTC().Format("2006-01-02T15:04:05.000Z") {
		return result, commercial.ErrPaidFulfillmentIntegrity
	}
	if err := result.Validate(); err != nil {
		return result, paidIntegrity(err)
	}
	return result, nil
}

func validatePaidRedeliverySource(ctx context.Context, tx *sql.Tx, record commercial.PaidRedeliveryRecord) error {
	fulfillment, err := scanPaidFulfillment(tx.QueryRowContext(ctx, paidFulfillmentSelect+" WHERE id=?", record.Snapshot.FulfillmentID))
	if err != nil {
		return paidIntegrity(err)
	}
	if err := validatePaidSources(ctx, tx, fulfillment); err != nil {
		return err
	}
	return record.ValidateFulfillment(fulfillment)
}

func (s *Store) getPaidRedelivery(ctx context.Context, predicate, value string) (commercial.PaidRedeliveryRecord, error) {
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable, ReadOnly: true})
	if err != nil {
		return commercial.PaidRedeliveryRecord{}, err
	}
	defer tx.Rollback()
	record, err := scanPaidRedelivery(tx.QueryRowContext(ctx, paidRedeliverySelect+predicate, value))
	if errors.Is(err, sql.ErrNoRows) {
		return record, commercial.ErrNotFound
	}
	if err != nil {
		return record, err
	}
	if err := validatePaidRedeliverySource(ctx, tx, record); err != nil {
		return record, err
	}
	return record, tx.Commit()
}

func (s *Store) GetPaidRedelivery(ctx context.Context, id string) (commercial.PaidRedeliveryRecord, error) {
	return s.getPaidRedelivery(ctx, " WHERE id=?", id)
}

func (s *Store) RecordPaidRedelivery(ctx context.Context, id, fulfillmentID string, input commercial.RecordPaidRedeliveryInput, environment, actor string) (commercial.PaidRedeliveryRecord, error) {
	requested, err := input.Bytes()
	if err != nil {
		return commercial.PaidRedeliveryRecord{}, err
	}
	old, err := s.getPaidRedelivery(ctx, " WHERE operation_id=?", input.OperationID)
	if err == nil {
		if !old.Matches(id, fulfillmentID, actor, requested) {
			return commercial.PaidRedeliveryRecord{}, commercial.ErrConflict
		}
		return old, nil
	}
	if !errors.Is(err, commercial.ErrNotFound) {
		return old, err
	}
	if environment != "local" && environment != "production" {
		return old, commercial.ErrFulfillmentEnvironment
	}

	var result commercial.PaidRedeliveryRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		old, err := scanPaidRedelivery(tx.QueryRowContext(ctx, paidRedeliverySelect+" WHERE operation_id=? FOR UPDATE", input.OperationID))
		if err == nil {
			if !old.Matches(id, fulfillmentID, actor, requested) {
				return commercial.ErrConflict
			}
			if err := validatePaidRedeliverySource(ctx, tx, old); err != nil {
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
		if fulfillment.Snapshot.Environment != environment {
			return commercial.ErrFulfillmentEnvironment
		}
		now := time.Now().UTC().Truncate(time.Millisecond)
		result, err = commercial.NewPaidRedelivery(id, input, fulfillment, actor, now)
		if err != nil {
			return err
		}
		snapshot, err := result.Snapshot.Bytes()
		if err != nil {
			return err
		}
		v := result.Snapshot
		if _, err := tx.ExecContext(ctx, `INSERT INTO commercial_paid_redeliveries(id,operation_id,fulfillment_id,fulfillment_sha256,order_id,customer_id,document_sha256,environment,snapshot_json,content_sha256,requested_by,requested_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)`, v.ID, v.Request.OperationID, v.FulfillmentID, v.FulfillmentSHA256, v.OrderID, v.CustomerID, v.DocumentSHA256, v.Environment, snapshot, result.SHA256, v.RequestedBy, now); err != nil {
			return commercialConflict(err)
		}
		if err := writeCommercialAudit(ctx, tx, input.OperationID, "commercial.fulfillment_redelivery_recorded", id, actor, result.SHA256, now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return result, err
}
