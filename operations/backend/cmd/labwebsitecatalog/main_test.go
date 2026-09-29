package main

import (
	"encoding/json"
	"os"
	"testing"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/productcatalog"
)

func freeInput(t *testing.T) input {
	t.Helper()
	raw, err := os.ReadFile("../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var def commercial.Definition
	if err := json.Unmarshal(raw, &def); err != nil {
		t.Fatal(err)
	}
	def.Offer = commercial.Offer{Kind: "free", Expiry: &licenseprotocol.ExpiryV2{Mode: licenseprotocol.NoExpiryV2}}
	for i, quota := range def.Entitlements.Quotas {
		values := map[productcatalog.QuotaID]uint32{productcatalog.QuotaMemberSeats: 3, productcatalog.QuotaRunners: 1, productcatalog.QuotaUpstreamAccounts: 1, productcatalog.QuotaApiKeysPerMember: 2}
		def.Entitlements.Quotas[i].Limit = productcatalog.Limited(values[quota.ID])
	}
	return input{PlanID: "test_bundled_free", Version: 7, Definition: def}
}

func TestReviewPreservesFreeIdentityAndCalculatesBothConfirmedSubscriptions(t *testing.T) {
	in := freeInput(t)
	out, err := prepare(in)
	if err != nil {
		t.Fatal(err)
	}
	if out.Catalog.Environment != "local" || len(out.Catalog.Plans) != 4 || len(out.Plans) != 4 {
		t.Fatal("not a complete local review")
	}
	free := out.Catalog.Plans[0]
	if free.PlanID != in.PlanID || free.Version != in.Version || len(free.Entitlements.FeatureSets) != 0 {
		t.Fatal("bundled identity or explicit rights changed")
	}
	for _, quota := range free.Entitlements.Quotas {
		if quota.ID == productcatalog.QuotaMemberSeats && (quota.Limit.Value == nil || *quota.Limit.Value != 3) {
			t.Fatal("free seats changed")
		}
	}
	expected := [][]int64{{599900, 1079820, 1529745, 1919680, 2099650}, {999900, 1799820, 2549745, 3199680, 3499650}}
	for i, amounts := range expected {
		plan := out.Catalog.Plans[i+1]
		if len(plan.Entitlements.Features) != 0 || len(plan.Entitlements.FeatureSets) != 1 || plan.Entitlements.FeatureSets[0] != productcatalog.FeatureSetStandard {
			t.Fatal("subscription lost symbolic standard rights")
		}
		if plan.Offer.TaxMode != "none" || len(plan.Offer.Terms) != 5 {
			t.Fatal("invented tax status or missing terms")
		}
		for j, amount := range amounts {
			if plan.Offer.Terms[j].TotalAmountMinor != amount {
				t.Fatalf("wrong term total: %+v", plan.Offer.Terms[j])
			}
		}
	}
	if out.Catalog.Plans[3].Offer.Kind != "contact" || out.Catalog.Plans[3].Offer.AnnualAmountMinor != 0 {
		t.Fatal("invented custom price")
	}
}

func TestReviewRejectsInvalidFreeRights(t *testing.T) {
	in := freeInput(t)
	in.Definition.Entitlements.FeatureSets = []productcatalog.FeatureSetID{productcatalog.FeatureSetStandard}
	if _, err := prepare(in); err == nil {
		t.Fatal("free symbolic rights accepted")
	}
	in = freeInput(t)
	in.Definition.Entitlements.Quotas = nil
	if _, err := prepare(in); err == nil {
		t.Fatal("missing quotas accepted")
	}
}
