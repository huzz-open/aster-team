package commercial

import (
	"bytes"
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func TestQuotationOrderBindsSourceAndPreservesV1(t *testing.T) {
	p := acceptFixture(publicationFixture(t))
	now, _ := parseTime("2026-09-06T00:00:02.000Z")
	in := QuotationOrderInput{OperationID: "op", CustomerID: "customer", PublicationID: p.Snapshot.ID, CatalogRevision: p.Snapshot.Catalog.ID, PlanID: "plan_annual", PlanVersion: 1, Years: 1, StartsAt: "2026-09-07T00:00:00.000Z"}
	order, err := CreateQuotationOrderSnapshot("order", in, p, "local", now)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := order.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	parsed, err := ParseOrderSnapshot(raw, ContentDigest(raw))
	if err != nil || parsed.Source.PublicationSHA256 != p.SHA256 {
		t.Fatal("source lost", err)
	}
	for _, mutate := range []func(map[string]any){
		func(v map[string]any) { delete(v, "source") },
		func(v map[string]any) { v["source"] = nil },
		func(v map[string]any) { v["schema"] = OrderSchema },
		func(v map[string]any) { v["schema"] = "aster.order-snapshot.v3" },
		func(v map[string]any) { v["source"].(map[string]any)["ordered_at"] = p.Snapshot.Request.AcceptUntil },
		func(v map[string]any) { v["source"].(map[string]any)["environment"] = "production" },
		func(v map[string]any) { v["source"].(map[string]any)["unknown"] = true },
		func(v map[string]any) { v["amount_minor"] = 1 },
	} {
		var value map[string]any
		_ = json.Unmarshal(raw, &value)
		mutate(value)
		corrupt, _ := json.Marshal(value)
		var invalid OrderSnapshot
		if json.Unmarshal(corrupt, &invalid) == nil {
			t.Fatal("invalid snapshot accepted", string(corrupt))
		}
	}
	plan, _ := p.ResolveQuotation("local", in.CatalogRevision, in.PlanID, 1, 1, now)
	starts, _ := parseTime(in.StartsAt)
	legacy, _ := CreateOrderSnapshot("order", "customer", plan, 1, starts)
	legacyRaw, _ := legacy.Bytes()
	// Preserve the exact v1 representation: no source key or changed schema.
	var original map[string]any
	_ = json.Unmarshal(raw, &original)
	delete(original, "source")
	original["schema"] = OrderSchema
	originalRaw, _ := canonical(original)
	if !bytes.Equal(originalRaw, legacyRaw) {
		t.Fatal("v1 bytes changed")
	}
	if _, err := ParseOrderSnapshot(legacyRaw, ContentDigest(originalRaw)); err != nil {
		t.Fatal(err)
	}
	record := OrderRecord{Snapshot: order, SHA256: ContentDigest(raw), OperationID: in.OperationID, CreatedBy: "actor", CreatedAt: now}
	if !record.MatchesQuotationRequest("order", in, "actor") {
		t.Fatal("original recovery failed")
	}
	for _, mutate := range []func(*QuotationOrderInput){
		func(v *QuotationOrderInput) { v.PublicationID += "_other" },
		func(v *QuotationOrderInput) { v.CatalogRevision = "catalog_" + strings.Repeat("d", 48) },
		func(v *QuotationOrderInput) { v.PlanVersion++ },
		func(v *QuotationOrderInput) { v.Years++ },
		func(v *QuotationOrderInput) { v.CustomerID += "_other" },
		func(v *QuotationOrderInput) { v.StartsAt = now.Add(time.Hour).Format("2006-01-02T15:04:05.000Z") },
	} {
		changed := in
		mutate(&changed)
		if record.MatchesQuotationRequest("order", changed, "actor") {
			t.Fatal("changed request recovered")
		}
	}
}

func TestQuotationOrderInputRejectsClientAuthority(t *testing.T) {
	p := acceptFixture(publicationFixture(t))
	in := QuotationOrderInput{OperationID: "op", CustomerID: "customer", PublicationID: p.Snapshot.ID, CatalogRevision: p.Snapshot.Catalog.ID, PlanID: "plan_annual", PlanVersion: 1, Years: 1, StartsAt: "2026-09-07T00:00:00.000Z"}
	raw, _ := json.Marshal(in)
	for _, field := range []string{"environment", "amount_minor", "source", "entitlements", "accepted_at"} {
		bad := append([]byte(`{"`+field+`":"forged",`), raw[1:]...)
		var decoded QuotationOrderInput
		if json.Unmarshal(bad, &decoded) == nil {
			t.Fatal("client authority accepted", field)
		}
	}
}
