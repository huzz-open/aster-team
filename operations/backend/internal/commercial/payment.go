package commercial

import (
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/strictjson"
)

const PaymentSchema = "aster.payment-confirmation.v1"

var ErrInvalidPayment = errors.New("invalid commercial payment confirmation")
var ErrPaymentIntegrity = errors.New("stored commercial payment integrity failure")

// This records an operator's confirmation of full receipt, not a payment gateway
// charge. The expected digest binds the displayed amount and all purchased terms.
type ConfirmPaymentInput struct {
	OperationID         string `json:"operation_id"`
	ExpectedOrderSHA256 string `json:"expected_order_sha256"`
	PaymentReference    string `json:"payment_reference"`
	ReceivedAt          string `json:"received_at"`
	Notes               string `json:"notes"`
}

func (v *ConfirmPaymentInput) UnmarshalJSON(data []byte) error {
	type wire ConfirmPaymentInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "expected_order_sha256", "payment_reference", "received_at", "notes"); err != nil {
		return err
	}
	*v = ConfirmPaymentInput(result)
	return v.Validate()
}
func (v ConfirmPaymentInput) Validate() error {
	if !identifier.MatchString(v.OperationID) || !catalogDigest.MatchString(v.ExpectedOrderSHA256) || !validText(v.PaymentReference, 256, false) || !validText(v.Notes, 2000, true) {
		return ErrInvalidPayment
	}
	if _, err := parseTime(v.ReceivedAt); err != nil {
		return ErrInvalidPayment
	}
	return nil
}
func (v ConfirmPaymentInput) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	return canonical(v)
}

type PaymentSnapshot struct {
	Schema      string              `json:"schema"`
	ID          string              `json:"id"`
	Order       OrderSnapshot       `json:"order"`
	OrderSHA256 string              `json:"order_sha256"`
	Request     ConfirmPaymentInput `json:"request"`
	ConfirmedBy string              `json:"confirmed_by"`
	ConfirmedAt string              `json:"confirmed_at"`
}

func (v *PaymentSnapshot) UnmarshalJSON(data []byte) error {
	type wire PaymentSnapshot
	var result wire
	if err := strictjson.ObjectWithin(data, &result, 2<<20, "schema", "id", "order", "order_sha256", "request", "confirmed_by", "confirmed_at"); err != nil {
		return err
	}
	*v = PaymentSnapshot(result)
	return v.Validate()
}
func (v PaymentSnapshot) Validate() error {
	if v.Schema != PaymentSchema || !identifier.MatchString(v.ID) || len(v.ID) > 64 || !identifier.MatchString(v.ConfirmedBy) || len(v.ConfirmedBy) > 64 || v.Request.Validate() != nil {
		return ErrInvalidPayment
	}
	raw, err := v.Order.Bytes()
	if err != nil || ContentDigest(raw) != v.OrderSHA256 || v.Request.ExpectedOrderSHA256 != v.OrderSHA256 {
		return ErrInvalidPayment
	}
	confirmed, e1 := parseTime(v.ConfirmedAt)
	received, e2 := parseTime(v.Request.ReceivedAt)
	if e1 != nil || e2 != nil || received.After(confirmed) {
		return ErrInvalidPayment
	}
	if v.Order.Source != nil {
		ordered, _ := parseTime(v.Order.Source.OrderedAt)
		if ordered.After(confirmed) {
			return ErrInvalidPayment
		}
	}
	return nil
}
func (v PaymentSnapshot) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	raw, err := canonical(v)
	if len(raw) > 2<<20 {
		return nil, ErrInvalidPayment
	}
	return raw, err
}

type PaymentRecord struct {
	Snapshot PaymentSnapshot `json:"snapshot"`
	SHA256   string          `json:"sha256"`
}

func (v *PaymentRecord) UnmarshalJSON(data []byte) error {
	type wire PaymentRecord
	var result wire
	if err := strictjson.ObjectWithin(data, &result, 3<<20, "snapshot", "sha256"); err != nil {
		return err
	}
	*v = PaymentRecord(result)
	return v.Validate()
}
func (v PaymentRecord) Validate() error {
	raw, err := v.Snapshot.Bytes()
	if err != nil || ContentDigest(raw) != v.SHA256 {
		return ErrInvalidPayment
	}
	return nil
}

func NewPaymentRecord(id string, in ConfirmPaymentInput, order OrderRecord, actor string, now time.Time) (PaymentRecord, error) {
	if order.Status != "pending_payment" || in.ExpectedOrderSHA256 != order.SHA256 || now.Before(order.CreatedAt) {
		return PaymentRecord{}, ErrInvalidPayment
	}
	raw, err := order.Snapshot.Bytes()
	if err != nil {
		return PaymentRecord{}, err
	}
	copy, err := ParseOrderSnapshot(raw, order.SHA256)
	if err != nil {
		return PaymentRecord{}, err
	}
	snapshot := PaymentSnapshot{Schema: PaymentSchema, ID: id, Order: copy, OrderSHA256: order.SHA256, Request: in, ConfirmedBy: actor, ConfirmedAt: now.UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")}
	raw, err = snapshot.Bytes()
	if err != nil {
		return PaymentRecord{}, err
	}
	return PaymentRecord{Snapshot: snapshot, SHA256: ContentDigest(raw)}, nil
}

func ParsePaymentRecord(raw []byte, hash string) (PaymentRecord, error) {
	var snapshot PaymentSnapshot
	if err := json.Unmarshal(raw, &snapshot); err != nil {
		return PaymentRecord{}, err
	}
	r := PaymentRecord{Snapshot: snapshot, SHA256: hash}
	return r, r.Validate()
}
