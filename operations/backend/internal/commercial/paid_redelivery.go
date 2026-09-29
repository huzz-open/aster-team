package commercial

import (
	"bytes"
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/strictjson"
)

const PaidRedeliverySchema = "aster.paid-redelivery.v1"

var ErrInvalidPaidRedelivery = errors.New("invalid paid fulfillment redelivery")

type RecordPaidRedeliveryInput struct {
	OperationID            string `json:"operation_id"`
	ExpectedDocumentSHA256 string `json:"expected_document_sha256"`
	Reason                 string `json:"reason"`
}

func (v *RecordPaidRedeliveryInput) UnmarshalJSON(data []byte) error {
	type wire RecordPaidRedeliveryInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "expected_document_sha256", "reason"); err != nil {
		return err
	}
	*v = RecordPaidRedeliveryInput(result)
	return v.Validate()
}

func (v RecordPaidRedeliveryInput) Validate() error {
	if !identifier.MatchString(v.OperationID) || len(v.OperationID) > 128 || !catalogDigest.MatchString(v.ExpectedDocumentSHA256) || !validText(v.Reason, 2000, false) {
		return ErrInvalidPaidRedelivery
	}
	return nil
}

func (v RecordPaidRedeliveryInput) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	return canonical(v)
}

type PaidRedeliverySnapshot struct {
	Schema            string                    `json:"schema"`
	ID                string                    `json:"id"`
	FulfillmentID     string                    `json:"fulfillment_id"`
	FulfillmentSHA256 string                    `json:"fulfillment_sha256"`
	OrderID           string                    `json:"order_id"`
	CustomerID        string                    `json:"customer_id"`
	DocumentSHA256    string                    `json:"document_sha256"`
	Environment       string                    `json:"environment"`
	Request           RecordPaidRedeliveryInput `json:"request"`
	RequestedBy       string                    `json:"requested_by"`
	RequestedAt       string                    `json:"requested_at"`
}

func (v *PaidRedeliverySnapshot) UnmarshalJSON(data []byte) error {
	type wire PaidRedeliverySnapshot
	var result wire
	if err := strictjson.ObjectWithin(data, &result, 64<<10, "schema", "id", "fulfillment_id", "fulfillment_sha256", "order_id", "customer_id", "document_sha256", "environment", "request", "requested_by", "requested_at"); err != nil {
		return err
	}
	*v = PaidRedeliverySnapshot(result)
	return v.Validate()
}

func (v PaidRedeliverySnapshot) Validate() error {
	if v.Schema != PaidRedeliverySchema || !identifier.MatchString(v.ID) || len(v.ID) > 64 ||
		!identifier.MatchString(v.FulfillmentID) || len(v.FulfillmentID) > 64 ||
		!identifier.MatchString(v.OrderID) || len(v.OrderID) > 128 ||
		!identifier.MatchString(v.CustomerID) || len(v.CustomerID) > 64 ||
		!identifier.MatchString(v.RequestedBy) || len(v.RequestedBy) > 64 ||
		!catalogDigest.MatchString(v.FulfillmentSHA256) || !catalogDigest.MatchString(v.DocumentSHA256) ||
		v.Request.Validate() != nil || v.Request.ExpectedDocumentSHA256 != v.DocumentSHA256 ||
		(v.Environment != "local" && v.Environment != "production") {
		return ErrInvalidPaidRedelivery
	}
	if _, err := parseTime(v.RequestedAt); err != nil {
		return ErrInvalidPaidRedelivery
	}
	return nil
}

func (v PaidRedeliverySnapshot) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	raw, err := canonical(v)
	if len(raw) > 64<<10 {
		return nil, ErrInvalidPaidRedelivery
	}
	return raw, err
}

type PaidRedeliveryRecord struct {
	Snapshot PaidRedeliverySnapshot `json:"snapshot"`
	SHA256   string                 `json:"sha256"`
}

func (v *PaidRedeliveryRecord) UnmarshalJSON(data []byte) error {
	type wire PaidRedeliveryRecord
	var result wire
	if err := strictjson.ObjectWithin(data, &result, 128<<10, "snapshot", "sha256"); err != nil {
		return err
	}
	*v = PaidRedeliveryRecord(result)
	return v.Validate()
}

func (v PaidRedeliveryRecord) Validate() error {
	raw, err := v.Snapshot.Bytes()
	if err != nil || ContentDigest(raw) != v.SHA256 {
		return ErrInvalidPaidRedelivery
	}
	return nil
}

func (v PaidRedeliveryRecord) ValidateFulfillment(fulfillment PaidFulfillmentRecord) error {
	if v.Validate() != nil || fulfillment.Validate() != nil || fulfillment.Status != "issued" || fulfillment.Document == nil {
		return ErrPaidFulfillmentIntegrity
	}
	snapshot := v.Snapshot
	if snapshot.FulfillmentID != fulfillment.Snapshot.ID || snapshot.FulfillmentSHA256 != fulfillment.SHA256 ||
		snapshot.OrderID != fulfillment.Snapshot.Payment.Snapshot.Order.OrderID ||
		snapshot.CustomerID != fulfillment.Snapshot.Payment.Snapshot.Order.CustomerID ||
		snapshot.DocumentSHA256 != fulfillment.DocumentSHA256 || snapshot.Environment != fulfillment.Snapshot.Environment {
		return ErrPaidFulfillmentIntegrity
	}
	return nil
}

func NewPaidRedelivery(id string, input RecordPaidRedeliveryInput, fulfillment PaidFulfillmentRecord, actor string, now time.Time) (PaidRedeliveryRecord, error) {
	if input.Validate() != nil || fulfillment.Validate() != nil || fulfillment.Status != "issued" || fulfillment.Document == nil ||
		input.ExpectedDocumentSHA256 != fulfillment.DocumentSHA256 {
		return PaidRedeliveryRecord{}, ErrInvalidPaidRedelivery
	}
	snapshot := PaidRedeliverySnapshot{
		Schema: PaidRedeliverySchema, ID: id, FulfillmentID: fulfillment.Snapshot.ID, FulfillmentSHA256: fulfillment.SHA256,
		OrderID: fulfillment.Snapshot.Payment.Snapshot.Order.OrderID, CustomerID: fulfillment.Snapshot.Payment.Snapshot.Order.CustomerID,
		DocumentSHA256: fulfillment.DocumentSHA256, Environment: fulfillment.Snapshot.Environment, Request: input,
		RequestedBy: actor, RequestedAt: now.UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z"),
	}
	raw, err := snapshot.Bytes()
	if err != nil {
		return PaidRedeliveryRecord{}, err
	}
	var owned PaidRedeliverySnapshot
	if err := json.Unmarshal(raw, &owned); err != nil {
		return PaidRedeliveryRecord{}, err
	}
	record := PaidRedeliveryRecord{Snapshot: owned, SHA256: ContentDigest(raw)}
	if record.ValidateFulfillment(fulfillment) != nil {
		return PaidRedeliveryRecord{}, ErrInvalidPaidRedelivery
	}
	return record, nil
}

func (v PaidRedeliveryRecord) Matches(id, fulfillmentID, actor string, input []byte) bool {
	stored, err := v.Snapshot.Request.Bytes()
	return err == nil && v.Snapshot.ID == id && v.Snapshot.FulfillmentID == fulfillmentID &&
		v.Snapshot.RequestedBy == actor && bytes.Equal(stored, input)
}
