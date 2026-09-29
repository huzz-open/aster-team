package commercial

import (
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func TestPaymentBindsOriginalOrderAndStrictInput(t *testing.T) {
	p := acceptFixture(publicationFixture(t))
	now, _ := parseTime("2026-09-06T00:00:02.000Z")
	input := QuotationOrderInput{OperationID: "order_op", CustomerID: "customer", PublicationID: p.Snapshot.ID, CatalogRevision: p.Snapshot.Catalog.ID, PlanID: "plan_annual", PlanVersion: 1, Years: 1, StartsAt: "2026-09-07T00:00:00.000Z"}
	snapshot, err := CreateQuotationOrderSnapshot("order", input, p, "local", now)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := snapshot.Bytes()
	order := OrderRecord{Snapshot: snapshot, SHA256: ContentDigest(raw), Status: "pending_payment", CreatedAt: now, OperationID: input.OperationID, CreatedBy: "actor"}
	in := ConfirmPaymentInput{OperationID: "payment_op", ExpectedOrderSHA256: order.SHA256, PaymentReference: "bank_reference", ReceivedAt: now.Format("2006-01-02T15:04:05.000Z"), Notes: "full receipt"}
	// Payment may be documented after the quotation deadline, using its original
	// accepted time. A changed publication cannot supply the same order source.
	if err := order.ValidatePublicationSource(p); err != nil {
		t.Fatal(err)
	}
	other := p
	other.SHA256 = strings.Repeat("b", 64)
	if order.ValidatePublicationSource(other) == nil {
		t.Fatal("unrelated source admitted")
	}
	record, err := NewPaymentRecord("payment", in, order, "actor", now.AddDate(2, 0, 0))
	if err != nil {
		t.Fatal("late receipt rejected", err)
	}
	encoded, err := json.Marshal(record)
	if err != nil {
		t.Fatal(err)
	}
	var decoded PaymentRecord
	if err := json.Unmarshal(encoded, &decoded); err != nil || decoded.SHA256 != record.SHA256 {
		t.Fatal("round trip", err)
	}
	for _, mutate := range []func(*ConfirmPaymentInput){
		func(v *ConfirmPaymentInput) { v.ExpectedOrderSHA256 = strings.Repeat("f", 64) },
		func(v *ConfirmPaymentInput) { v.ReceivedAt = now.Add(3 * time.Hour).Format("2006-01-02T15:04:05.000Z") },
		func(v *ConfirmPaymentInput) { v.PaymentReference = " " },
	} {
		bad := in
		mutate(&bad)
		if _, err := NewPaymentRecord("payment", bad, order, "actor", now); err == nil {
			t.Fatal("invalid confirmation accepted")
		}
	}
	inputRaw, _ := json.Marshal(in)
	for _, bad := range []string{
		string(inputRaw) + "garbage", string(inputRaw) + "{}",
		strings.Replace(string(inputRaw), `"notes":`, `"notes":"duplicate","notes":`, 1),
		strings.Replace(string(inputRaw), `"notes":`, `"Notes":`, 1),
		strings.Replace(string(inputRaw), `"notes":"full receipt"`, `"notes":null`, 1),
		`{"amount_minor":1,` + string(inputRaw[1:]),
	} {
		var value ConfirmPaymentInput
		if json.Unmarshal([]byte(bad), &value) == nil {
			t.Fatal("noncanonical authority accepted", bad)
		}
	}
	changed := record
	changed.Snapshot.Request.Notes = "edited"
	if changed.Validate() == nil {
		t.Fatal("modified receipt admitted")
	}
}
