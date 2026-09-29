package commercial

import (
	"bytes"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/strictjson"
)

// Recheck the trusted original event at the original order instant. A compact
// receipt's own digest is not approval, and today's quote deadline is irrelevant.
func (v OrderRecord) ValidatePublicationSource(publication PublicationRecord) error {
	if v.Snapshot.Schema != QuotationOrderSchema || v.Snapshot.Source == nil {
		return ErrInvalidQuotationOrder
	}
	source := v.Snapshot.Source
	ordered, err := parseTime(source.OrderedAt)
	if err != nil {
		return err
	}
	input := QuotationOrderInput{OperationID: v.OperationID, CustomerID: v.Snapshot.CustomerID, PublicationID: source.PublicationID, CatalogRevision: source.CatalogRevision, PlanID: v.Snapshot.Plan.PlanID, PlanVersion: v.Snapshot.Plan.Version, Years: v.Snapshot.Years, StartsAt: v.Snapshot.StartsAt}
	expected, err := CreateQuotationOrderSnapshot(v.Snapshot.OrderID, input, publication, source.Environment, ordered)
	if err != nil {
		return err
	}
	want, err := expected.Bytes()
	if err != nil {
		return err
	}
	got, err := v.Snapshot.Bytes()
	if err != nil || !bytes.Equal(want, got) || ContentDigest(got) != v.SHA256 {
		return ErrInvalidQuotationOrder
	}
	return nil
}

var ErrInvalidQuotationOrder = errors.New("invalid quotation order")

// Minimal order-entry projection; no approval reasons, actors or failure history.
type QuotationSource struct {
	PublicationID   string         `json:"publication_id"`
	CatalogRevision string         `json:"catalog_revision"`
	Environment     string         `json:"environment"`
	AcceptUntil     string         `json:"accept_until"`
	Plans           []PlanSnapshot `json:"plans"`
}

func (v PublicationRecord) QuotationSource(environment string, now time.Time) (QuotationSource, error) {
	if err := v.Validate(); err != nil {
		return QuotationSource{}, err
	}
	accepted, _ := parseTime(v.AcceptedAt)
	until, _ := parseTime(v.Snapshot.Request.AcceptUntil)
	if v.Status != "accepted" || environment != v.Snapshot.Catalog.Request.Environment || now.Before(accepted) || !now.Before(until) {
		return QuotationSource{}, ErrPublicationNotAccepted
	}
	source := QuotationSource{PublicationID: v.Snapshot.ID, CatalogRevision: v.Snapshot.Catalog.ID, Environment: environment, AcceptUntil: v.Snapshot.Request.AcceptUntil, Plans: []PlanSnapshot{}}
	for _, plan := range v.Snapshot.Catalog.Plans {
		if plan.Definition.Offer.Kind == "annual" {
			source.Plans = append(source.Plans, plan)
		}
	}
	return source, nil
}

// A visitor's reference is only a selection. No price, rights, channel or
// acceptance evidence can be supplied by the caller.
type QuotationOrderInput struct {
	OperationID     string `json:"operation_id"`
	CustomerID      string `json:"customer_id"`
	PublicationID   string `json:"publication_id"`
	CatalogRevision string `json:"catalog_revision"`
	PlanID          string `json:"plan_id"`
	PlanVersion     uint32 `json:"plan_version"`
	Years           uint32 `json:"years"`
	StartsAt        string `json:"starts_at"`
}

func (v *QuotationOrderInput) UnmarshalJSON(data []byte) error {
	type wire QuotationOrderInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "customer_id", "publication_id", "catalog_revision", "plan_id", "plan_version", "years", "starts_at"); err != nil {
		return err
	}
	*v = QuotationOrderInput(result)
	return v.Validate()
}
func (v QuotationOrderInput) Validate() error {
	if !identifier.MatchString(v.OperationID) || !catalogRevision.MatchString(v.CatalogRevision) || v.PlanVersion == 0 || v.Years < 1 || v.Years > 5 {
		return ErrInvalidQuotationOrder
	}
	for _, id := range []string{v.CustomerID, v.PublicationID, v.PlanID} {
		if !identifier.MatchString(id) || len(id) > 64 {
			return ErrInvalidQuotationOrder
		}
	}
	if _, err := parseTime(v.StartsAt); err != nil {
		return ErrInvalidQuotationOrder
	}
	return nil
}

// This immutable receipt and the selected full plan are inside the order's
// digest. Other catalog plans do not need to be copied into every order.
type QuotationOrderSource struct {
	Kind              string `json:"kind"`
	PublicationID     string `json:"publication_id"`
	PublicationSHA256 string `json:"publication_sha256"`
	Environment       string `json:"environment"`
	CatalogRevision   string `json:"catalog_revision"`
	CatalogSHA256     string `json:"catalog_sha256"`
	BuildSHA256       string `json:"build_sha256"`
	Origin            string `json:"origin"`
	AcceptedAt        string `json:"accepted_at"`
	AcceptUntil       string `json:"accept_until"`
	OrderedAt         string `json:"ordered_at"`
}

func (v *QuotationOrderSource) UnmarshalJSON(data []byte) error {
	type wire QuotationOrderSource
	var result wire
	if err := strictjson.Object(data, &result, "kind", "publication_id", "publication_sha256", "environment", "catalog_revision", "catalog_sha256", "build_sha256", "origin", "accepted_at", "accept_until", "ordered_at"); err != nil {
		return err
	}
	*v = QuotationOrderSource(result)
	return v.Validate()
}
func (v QuotationOrderSource) Validate() error {
	if v.Kind != "publication" || !identifier.MatchString(v.PublicationID) || len(v.PublicationID) > 64 || !catalogRevision.MatchString(v.CatalogRevision) || ValidatePublicationOrigin(v.Environment, v.Origin) != nil {
		return ErrInvalidQuotationOrder
	}
	for _, hash := range []string{v.PublicationSHA256, v.CatalogSHA256, v.BuildSHA256} {
		if !catalogDigest.MatchString(hash) {
			return ErrInvalidQuotationOrder
		}
	}
	accepted, e1 := parseTime(v.AcceptedAt)
	until, e2 := parseTime(v.AcceptUntil)
	ordered, e3 := parseTime(v.OrderedAt)
	if e1 != nil || e2 != nil || e3 != nil || ordered.Before(accepted) || !ordered.Before(until) {
		return ErrInvalidQuotationOrder
	}
	return nil
}

func CreateQuotationOrderSnapshot(id string, input QuotationOrderInput, publication PublicationRecord, environment string, now time.Time) (OrderSnapshot, error) {
	if input.Validate() != nil || input.PublicationID != publication.Snapshot.ID || now.Nanosecond()%1_000_000 != 0 {
		return OrderSnapshot{}, ErrInvalidQuotationOrder
	}
	plan, err := publication.ResolveQuotation(environment, input.CatalogRevision, input.PlanID, input.PlanVersion, input.Years, now)
	if err != nil {
		return OrderSnapshot{}, err
	}
	starts, _ := parseTime(input.StartsAt)
	order, err := CreateOrderSnapshot(id, input.CustomerID, plan, input.Years, starts)
	if err != nil {
		return OrderSnapshot{}, err
	}
	order.Schema = QuotationOrderSchema
	order.Source = &QuotationOrderSource{
		Kind: "publication", PublicationID: publication.Snapshot.ID, PublicationSHA256: publication.SHA256,
		Environment: environment, CatalogRevision: input.CatalogRevision, CatalogSHA256: publication.Snapshot.Catalog.PublicSHA256,
		BuildSHA256: publication.Snapshot.Request.BuildSHA256, Origin: publication.Evidence.Origin,
		AcceptedAt: publication.AcceptedAt, AcceptUntil: publication.Snapshot.Request.AcceptUntil,
		OrderedAt: now.UTC().Format("2006-01-02T15:04:05.000Z"),
	}
	return order, order.Validate()
}

// Recovery compares the original request, not today's channel, catalog head,
// deadline or customer status. It never creates or extends an entitlement.
func (v OrderRecord) MatchesQuotationRequest(id string, input QuotationOrderInput, actor string) bool {
	s := v.Snapshot
	return s.Schema == QuotationOrderSchema && s.Source != nil && v.OperationID == input.OperationID && v.CreatedBy == actor && s.OrderID == id && s.CustomerID == input.CustomerID && s.Source.PublicationID == input.PublicationID && s.Source.CatalogRevision == input.CatalogRevision && s.Plan.PlanID == input.PlanID && s.Plan.Version == input.PlanVersion && s.Years == input.Years && s.StartsAt == input.StartsAt
}
