package mariadb

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"fmt"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

const paymentSelect = `SELECT id,operation_id,order_id,order_sha256,snapshot_json,content_sha256,confirmed_by,confirmed_at FROM commercial_payment_confirmations`

func scanPayment(row commercialScanner) (commercial.PaymentRecord, error) {
	var id, operation, order, orderHash, hash, actor string
	var raw []byte
	var confirmed time.Time
	if err := row.Scan(&id, &operation, &order, &orderHash, &raw, &hash, &actor, &confirmed); err != nil {
		return commercial.PaymentRecord{}, err
	}
	r, err := commercial.ParsePaymentRecord(raw, hash)
	if err != nil {
		return r, fmt.Errorf("%w: %v", commercial.ErrPaymentIntegrity, err)
	}
	v := r.Snapshot
	canonical, err := v.Bytes()
	if err != nil || !bytes.Equal(raw, canonical) || v.ID != id || v.Request.OperationID != operation || v.Order.OrderID != order || v.OrderSHA256 != orderHash || v.ConfirmedBy != actor || v.ConfirmedAt != confirmed.UTC().Format("2006-01-02T15:04:05.000Z") {
		return commercial.PaymentRecord{}, commercial.ErrPaymentIntegrity
	}
	return r, nil
}

// Called while holding the order lock. Existing quoted terms are checked at
// their ordered_at; an expired website quote never invalidates an existing sale.
func validateCommercialOrderSource(ctx context.Context, tx *sql.Tx, order commercial.OrderRecord) error {
	if order.Snapshot.Schema == commercial.QuotationOrderSchema {
		publication, err := scanPublication(tx.QueryRowContext(ctx, publicationSelect+" WHERE id=?", order.Snapshot.Source.PublicationID))
		if err != nil {
			return err
		}
		return order.ValidatePublicationSource(publication)
	}
	plan, err := scanCommercialPlan(tx.QueryRowContext(ctx, commercialPlanSelect+" WHERE plan_id=? AND version_no=?", order.Snapshot.Plan.PlanID, order.Snapshot.Plan.Version))
	if err != nil {
		return err
	}
	if plan.SHA256 != order.Snapshot.PlanSHA256 {
		return commercial.ErrConflict
	}
	return nil
}

// Restore receipts against the original immutable order, without requiring its
// current lifecycle state or today's quotation eligibility.
func validatePaymentOrder(ctx context.Context, tx *sql.Tx, payment commercial.PaymentRecord) error {
	order, err := scanCommercialOrder(tx.QueryRowContext(ctx, commercialOrderSelect+" WHERE id=?", payment.Snapshot.Order.OrderID))
	if err != nil {
		return fmt.Errorf("%w: original order: %v", commercial.ErrPaymentIntegrity, err)
	}
	expected, err := payment.Snapshot.Order.Bytes()
	if err != nil {
		return commercial.ErrPaymentIntegrity
	}
	actual, err := order.Snapshot.Bytes()
	if err != nil || order.SHA256 != payment.Snapshot.OrderSHA256 || !bytes.Equal(expected, actual) {
		return commercial.ErrPaymentIntegrity
	}
	confirmed, err := time.Parse(time.RFC3339Nano, payment.Snapshot.ConfirmedAt)
	if err != nil || order.CreatedAt.After(confirmed) {
		return commercial.ErrPaymentIntegrity
	}
	if err := validateCommercialOrderSource(ctx, tx, order); err != nil {
		return fmt.Errorf("%w: order source: %v", commercial.ErrPaymentIntegrity, err)
	}
	return nil
}
func (s *Store) GetCommercialPayment(ctx context.Context, orderID string) (commercial.PaymentRecord, error) {
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable, ReadOnly: true})
	if err != nil {
		return commercial.PaymentRecord{}, err
	}
	defer tx.Rollback()
	r, err := scanPayment(tx.QueryRowContext(ctx, paymentSelect+" WHERE order_id=?", orderID))
	if errors.Is(err, sql.ErrNoRows) {
		return r, commercial.ErrNotFound
	}
	if err != nil {
		return r, err
	}
	if err := validatePaymentOrder(ctx, tx, r); err != nil {
		return commercial.PaymentRecord{}, err
	}
	return r, tx.Commit()
}

func (s *Store) ConfirmCommercialPayment(ctx context.Context, id, orderID string, in commercial.ConfirmPaymentInput, actor string) (commercial.PaymentRecord, error) {
	requested, err := in.Bytes()
	if err != nil {
		return commercial.PaymentRecord{}, err
	}
	var result commercial.PaymentRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		old, err := scanPayment(tx.QueryRowContext(ctx, paymentSelect+" WHERE operation_id=? FOR UPDATE", in.OperationID))
		if err == nil {
			stored, err := old.Snapshot.Request.Bytes()
			if err != nil {
				return err
			}
			if old.Snapshot.ID != id || old.Snapshot.Order.OrderID != orderID || old.Snapshot.ConfirmedBy != actor || !bytes.Equal(requested, stored) {
				return commercial.ErrConflict
			}
			if err := validatePaymentOrder(ctx, tx, old); err != nil {
				return err
			}
			result = old
			return tx.Commit()
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		order, err := scanCommercialOrder(tx.QueryRowContext(ctx, commercialOrderSelect+" WHERE id=? FOR UPDATE", orderID))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrNotFound
		}
		if err != nil {
			return err
		}
		if order.Status != "pending_payment" || order.SHA256 != in.ExpectedOrderSHA256 {
			return commercial.ErrConflict
		}
		if err := validateCommercialOrderSource(ctx, tx, order); err != nil {
			return fmt.Errorf("%w: order source: %v", commercial.ErrPaymentIntegrity, err)
		}
		now := time.Now().UTC().Truncate(time.Millisecond)
		result, err = commercial.NewPaymentRecord(id, in, order, actor, now)
		if err != nil {
			return err
		}
		raw, err := result.Snapshot.Bytes()
		if err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO commercial_payment_confirmations(id,operation_id,order_id,order_sha256,snapshot_json,content_sha256,confirmed_by,confirmed_at) VALUES(?,?,?,?,?,?,?,?)`, id, in.OperationID, orderID, order.SHA256, raw, result.SHA256, actor, now); err != nil {
			return commercialConflict(err)
		}
		if _, err := tx.ExecContext(ctx, "UPDATE commercial_orders SET status='fulfillment_pending',updated_at=? WHERE id=? AND status='pending_payment'", now, orderID); err != nil {
			return err
		}
		if err := writeCommercialAudit(ctx, tx, in.OperationID, "commercial.payment_confirmed", id, actor, result.SHA256, now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return result, err
}
