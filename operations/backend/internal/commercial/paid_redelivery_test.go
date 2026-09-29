package commercial

import (
	"crypto/ed25519"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/licenseprotocol"
)

func issuedPaidFulfillmentFixture(t *testing.T) PaidFulfillmentRecord {
	t.Helper()
	fulfillment, _, now := paidFulfillmentFixture(t)
	claims, err := fulfillment.Snapshot.Claims("paid_test_key", now)
	if err != nil {
		t.Fatal(err)
	}
	document, err := licenseprotocol.SignV2(claims, ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize)))
	if err != nil {
		t.Fatal(err)
	}
	fulfillment.Status, fulfillment.Claims, fulfillment.Document = "issued", &claims, &document
	raw, err := json.Marshal(document)
	if err != nil {
		t.Fatal(err)
	}
	fulfillment.DocumentSHA256 = ContentDigest(raw)
	if err := fulfillment.Validate(); err != nil {
		t.Fatal(err)
	}
	return fulfillment
}

func TestPaidRedeliveryPreservesExactIssuedDocumentIdentity(t *testing.T) {
	fulfillment := issuedPaidFulfillmentFixture(t)
	input := RecordPaidRedeliveryInput{OperationID: "redeliver_1", ExpectedDocumentSHA256: fulfillment.DocumentSHA256, Reason: "customer lost the original file"}
	record, err := NewPaidRedelivery("redelivery_1", input, fulfillment, "operator_1", time.Date(2026, 9, 7, 10, 0, 0, 123456789, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	if record.Snapshot.RequestedAt != "2026-09-07T10:00:00.123Z" || record.Snapshot.DocumentSHA256 != fulfillment.DocumentSHA256 || record.ValidateFulfillment(fulfillment) != nil {
		t.Fatal("redelivery lost the original document identity")
	}
	raw, _ := json.Marshal(record)
	var decoded PaidRedeliveryRecord
	if err := json.Unmarshal(raw, &decoded); err != nil || decoded.ValidateFulfillment(fulfillment) != nil {
		t.Fatal("redelivery round trip", err)
	}
	inputBytes, err := input.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if !decoded.Matches("redelivery_1", fulfillment.Snapshot.ID, "operator_1", inputBytes) {
		t.Fatal("same operation did not match")
	}
}

func TestPaidRedeliveryRejectsChangesAndUnissuedSources(t *testing.T) {
	fulfillment := issuedPaidFulfillmentFixture(t)
	input := RecordPaidRedeliveryInput{OperationID: "redeliver_1", ExpectedDocumentSHA256: fulfillment.DocumentSHA256, Reason: "customer lost the original file"}
	record, err := NewPaidRedelivery("redelivery_1", input, fulfillment, "operator_1", time.Now())
	if err != nil {
		t.Fatal(err)
	}
	changed := fulfillment
	changed.DocumentSHA256 = strings.Repeat("0", 64)
	if record.ValidateFulfillment(changed) == nil {
		t.Fatal("changed document identity accepted")
	}
	fulfillment.Status, fulfillment.Document, fulfillment.DocumentSHA256 = "prepared", nil, ""
	if _, err := NewPaidRedelivery("redelivery_2", input, fulfillment, "operator_1", time.Now()); err == nil {
		t.Fatal("unissued fulfillment accepted")
	}
	var duplicate RecordPaidRedeliveryInput
	if err := json.Unmarshal([]byte(`{"operation_id":"a","operation_id":"b","expected_document_sha256":"`+strings.Repeat("0", 64)+`","reason":"x"}`), &duplicate); err == nil {
		t.Fatal("duplicate operation ID accepted")
	}
}
