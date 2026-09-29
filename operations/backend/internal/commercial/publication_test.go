package commercial

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"
)

func publicationFixture(t *testing.T) PublicationRecord {
	t.Helper()
	request, plans := catalogFixture(t)
	revision := "catalog_" + strings.Repeat("a", 48)
	preview, err := BuildPublicCatalog(revision, request, plans)
	if err != nil {
		t.Fatal(err)
	}
	now := "2026-09-06T00:00:00.000Z"
	snapshot := PublicationSnapshot{Schema: PublicationSchema, ID: "publication_test", Request: PreparePublicationInput{OperationID: "publication_test", CatalogRevision: revision, BuildSHA256: strings.Repeat("b", 64), AcceptUntil: "2026-10-01T00:00:00.000Z", Reason: "isolated publication"},
		Catalog: CatalogApprovalSnapshot{Schema: CatalogApprovalSchema, ID: revision, Request: request, Plans: plans, PublicSHA256: preview.SHA256, ApprovedBy: "operator_test", ApprovedAt: now}, CreatedBy: "operator_test", CreatedAt: now}
	raw, err := snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	r := PublicationRecord{Snapshot: snapshot, SHA256: ContentDigest(raw), Status: "prepared"}
	if err := r.Validate(); err != nil {
		t.Fatal(err)
	}
	return r
}

func acceptFixture(r PublicationRecord) PublicationRecord {
	r.Status, r.AcceptedBy, r.AcceptedAt = "accepted", "operator_acceptor", "2026-09-06T00:00:01.000Z"
	r.Evidence = &PublicationEvidence{Environment: "local", Origin: "http://127.0.0.1:26990", BuildSHA256: r.Snapshot.Request.BuildSHA256, CatalogRevision: r.Snapshot.Catalog.ID, CatalogSHA256: r.Snapshot.Catalog.PublicSHA256, ObservedAt: r.AcceptedAt}
	return r
}

func TestPublicationRequiresEvidenceAndExactPublicSelection(t *testing.T) {
	r := publicationFixture(t)
	now, _ := parseTime("2026-09-06T00:00:02.000Z")
	if _, err := r.ResolveQuotation("local", r.Snapshot.Catalog.ID, "plan_annual", 1, 1, now); !errors.Is(err, ErrPublicationNotAccepted) {
		t.Fatal("prepared event authorized a quote", err)
	}
	r = acceptFixture(r)
	if err := r.Validate(); err != nil {
		t.Fatal(err)
	}
	plan := r.Snapshot.Catalog.Plans[0]
	years := plan.Definition.Offer.Terms[0].Years
	for _, test := range []struct {
		name, environment, revision, plan string
		version, years                    uint32
		now                               time.Time
	}{
		{"local cannot authorize production", "production", r.Snapshot.Catalog.ID, "plan_annual", 1, years, now},
		{"revision splice", "local", "catalog_" + strings.Repeat("c", 48), "plan_annual", 1, years, now},
		{"hidden plan", "local", r.Snapshot.Catalog.ID, "hidden", 1, years, now},
		{"wrong version", "local", r.Snapshot.Catalog.ID, "plan_annual", 2, years, now},
		{"unpublished term", "local", r.Snapshot.Catalog.ID, "plan_annual", 1, 6, now},
		{"contact is not zero price", "local", r.Snapshot.Catalog.ID, "plan_contact", 1, years, now},
		{"free is not paid source", "local", r.Snapshot.Catalog.ID, "plan_free", 1, years, now},
		{"not yet accepted", "local", r.Snapshot.Catalog.ID, "plan_annual", 1, years, now.Add(-time.Hour)},
		{"deadline exclusive", "local", r.Snapshot.Catalog.ID, "plan_annual", 1, years, time.Date(2026, 10, 1, 0, 0, 0, 0, time.UTC)},
	} {
		t.Run(test.name, func(t *testing.T) {
			if _, err := r.ResolveQuotation(test.environment, test.revision, test.plan, test.version, test.years, test.now); !errors.Is(err, ErrPublicationNotAccepted) {
				t.Fatal("unaccepted reference passed", err)
			}
		})
	}
	frozen, err := r.ResolveQuotation("local", r.Snapshot.Catalog.ID, plan.PlanID, plan.Version, years, now)
	if err != nil {
		t.Fatal(err)
	}
	// Returned rights own their nested values; later edits cannot change the source.
	copy, err := frozen.Snapshot()
	if err != nil {
		t.Fatal(err)
	}
	copy.Definition.Entitlements.Features[0] = "not-a-real-capability"
	if err := r.Validate(); err != nil {
		t.Fatal("returned plan mutated the publication", err)
	}
}

func TestPublicationRejectsMixedStaleAndForgedEvidence(t *testing.T) {
	for _, mutate := range []func(*PublicationRecord){
		func(r *PublicationRecord) { r.Evidence.Environment = "production" },
		func(r *PublicationRecord) { r.Evidence.CatalogRevision = "catalog_" + strings.Repeat("d", 48) },
		func(r *PublicationRecord) { r.Evidence.BuildSHA256 = strings.Repeat("e", 64) },
		func(r *PublicationRecord) { r.Evidence.CatalogSHA256 = strings.Repeat("e", 64) },
		func(r *PublicationRecord) { r.Evidence.Origin = "https://example.com" },
		func(r *PublicationRecord) { r.Evidence.ObservedAt = "2026-09-05T23:59:59.000Z" },
		func(r *PublicationRecord) { r.Evidence.ObservedAt = "2026-09-06T00:00:02.000Z" },
		func(r *PublicationRecord) { r.AcceptedAt = "2026-09-06T00:02:00.000Z" },
		func(r *PublicationRecord) { r.Status = "prepared" },
	} {
		r := acceptFixture(publicationFixture(t))
		mutate(&r)
		if err := r.Validate(); err == nil {
			t.Fatal("invalid publication accepted")
		}
	}
	r := publicationFixture(t)
	raw, _ := r.Snapshot.Request.Bytes()
	for _, corrupt := range []string{strings.Replace(string(raw), `"expected_active_id":"",`, "", 1), strings.Replace(string(raw), `"operation_id":`, `"unknown":true,"operation_id":`, 1)} {
		var input PreparePublicationInput
		if err := json.Unmarshal([]byte(corrupt), &input); err == nil {
			t.Fatal("non-strict publication accepted", corrupt)
		}
	}
}

func TestPublicationRoundTripsNearMaximumCatalogSize(t *testing.T) {
	r := publicationFixture(t)
	r.Snapshot.Request.Reason = strings.Repeat("文", 1000)
	r.Snapshot.Catalog.Plans = []PlanSnapshot{}
	r.Snapshot.Catalog.Request.Plans = []CatalogSelection{}
	definition := testDefinition(t)
	setLast := func(characters int) error {
		i := len(r.Snapshot.Catalog.Plans) - 1
		d := definition
		d.Code = fmt.Sprintf("large_%d", i)
		d.Description = strings.Repeat("文", characters)
		f, err := FreezePlan(fmt.Sprintf("large_%d", i), 1, d)
		if err != nil {
			return err
		}
		p, err := f.Snapshot()
		if err != nil {
			return err
		}
		r.Snapshot.Catalog.Plans[i] = p
		r.Snapshot.Catalog.Request.Plans[i] = CatalogSelection{PlanID: p.PlanID, Version: 1, ExpectedSHA256: f.Digest()}
		preview, err := BuildPublicCatalog(r.Snapshot.Catalog.ID, r.Snapshot.Catalog.Request, r.Snapshot.Catalog.Plans)
		if err != nil {
			return err
		}
		r.Snapshot.Catalog.PublicSHA256 = preview.SHA256
		_, err = r.Snapshot.Catalog.Bytes()
		return err
	}
	for i := 0; i < 100; i++ {
		r.Snapshot.Catalog.Plans = append(r.Snapshot.Catalog.Plans, PlanSnapshot{})
		r.Snapshot.Catalog.Request.Plans = append(r.Snapshot.Catalog.Request.Plans, CatalogSelection{})
		if err := setLast(4000); err != nil {
			break
		}
	}
	low, high := 0, 4000
	for low < high {
		mid := (low + high + 1) / 2
		if err := setLast(mid); err == nil {
			low = mid
		} else {
			high = mid - 1
		}
	}
	if err := setLast(low); err != nil {
		t.Fatal(err)
	}
	catalogBytes, err := r.Snapshot.Catalog.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if len(catalogBytes) < (1<<20)-4 {
		t.Fatalf("test did not reach catalog boundary: %d", len(catalogBytes))
	}
	raw, err := r.Snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if len(raw) <= 1<<20 || len(raw) > MaximumPublicationBytes {
		t.Fatalf("not an oversized envelope around a valid catalog: %d", len(raw))
	}
	var decoded PublicationSnapshot
	if err := json.Unmarshal(raw, &decoded); err != nil {
		t.Fatal("saved snapshot cannot be read", err)
	}
	again, err := decoded.Bytes()
	if err != nil || string(raw) != string(again) {
		t.Fatal("publication roundtrip changed bytes", err)
	}
	if err := json.Unmarshal([]byte("{"+strings.Repeat(" ", MaximumPublicationBytes)+string(raw)[1:]), &decoded); err == nil {
		t.Fatal("unbounded envelope accepted")
	}
}
