package commercial

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"slices"

	"aster.local/team/operations/backend/internal/productcatalog"
	"aster.local/team/operations/backend/internal/strictjson"
)

const PublicCatalogSchema = "aster.public-plans.v1"
const CatalogApprovalSchema = "aster.catalog-approval.v1"

var catalogRevision = regexp.MustCompile(`^catalog_[a-f0-9]{48}$`)
var catalogDigest = regexp.MustCompile(`^[a-f0-9]{64}$`)
var ErrInvalidCatalog = errors.New("invalid public catalog")
var ErrCatalogPreviewConflict = fmt.Errorf("%w: reviewed public catalog differs before approval was stored", ErrConflict)

type CatalogSelection struct {
	PlanID         string `json:"plan_id"`
	Version        uint32 `json:"version"`
	ExpectedSHA256 string `json:"expected_sha256"`
}

func (v *CatalogSelection) UnmarshalJSON(data []byte) error {
	type wire CatalogSelection
	var result wire
	if err := strictjson.Object(data, &result, "plan_id", "version", "expected_sha256"); err != nil {
		return err
	}
	*v = CatalogSelection(result)
	return nil
}

// Selection order is the public display order. Membership, not historical plan
// count or a mutable visibility flag, defines the complete public collection.
type CatalogRequest struct {
	OperationID string             `json:"operation_id"`
	Environment string             `json:"environment"`
	Reason      string             `json:"reason"`
	Plans       []CatalogSelection `json:"plans"`
}

func (v *CatalogRequest) UnmarshalJSON(data []byte) error {
	type wire CatalogRequest
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "environment", "reason", "plans"); err != nil {
		return err
	}
	*v = CatalogRequest(result)
	return v.Validate()
}
func (v CatalogRequest) Validate() error {
	if !identifier.MatchString(v.OperationID) || (v.Environment != "local" && v.Environment != "production") || !validText(v.Reason, 1000, false) || v.Plans == nil || len(v.Plans) > 100 {
		return ErrInvalidCatalog
	}
	seen := map[string]bool{}
	for _, p := range v.Plans {
		if !identifier.MatchString(p.PlanID) || len(p.PlanID) > 64 || p.Version == 0 || !catalogDigest.MatchString(p.ExpectedSHA256) || seen[p.PlanID] {
			return ErrInvalidCatalog
		}
		seen[p.PlanID] = true
	}
	return nil
}
func (v CatalogRequest) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	return canonical(v)
}

// Dedicated nested public DTOs deliberately do not embed internal plan, order,
// operator, support, license or backup records. Adding internal fields cannot
// accidentally extend the public document.
type PublicQuotaLimit struct {
	Mode  string  `json:"mode"`
	Value *uint32 `json:"value,omitempty"`
}
type PublicQuota struct {
	ID    productcatalog.QuotaID `json:"id"`
	Limit PublicQuotaLimit       `json:"limit"`
}
type PublicEntitlements struct {
	FeatureSets    []productcatalog.FeatureSetID `json:"feature_sets,omitempty"`
	CatalogVersion uint32                        `json:"catalog_version"`
	Features       []productcatalog.CapabilityID `json:"features"`
	Quotas         []PublicQuota                 `json:"quotas"`
}
type PublicTerm struct {
	Years               uint32 `json:"years"`
	DiscountBasisPoints uint32 `json:"discount_basis_points"`
	TotalAmountMinor    int64  `json:"total_amount_minor"`
}
type PublicExpiry struct {
	Mode      string `json:"mode"`
	ExpiresAt string `json:"expires_at,omitempty"`
}
type PublicOffer struct {
	Kind              string        `json:"kind"`
	Currency          string        `json:"currency,omitempty"`
	AnnualAmountMinor int64         `json:"annual_amount_minor,omitempty"`
	TaxMode           string        `json:"tax_mode,omitempty"`
	Terms             []PublicTerm  `json:"terms,omitempty"`
	TermRule          string        `json:"term_rule,omitempty"`
	TermTimezone      string        `json:"term_timezone,omitempty"`
	Expiry            *PublicExpiry `json:"expiry,omitempty"`
}
type PublicPlan struct {
	PlanID              string             `json:"plan_id"`
	Version             uint32             `json:"version"`
	Name                string             `json:"name"`
	Description         string             `json:"description"`
	Edition             string             `json:"edition"`
	Entitlements        PublicEntitlements `json:"entitlements"`
	QuotaPolicyVersion  uint32             `json:"quota_policy_version"`
	MinimumVersion      string             `json:"minimum_version"`
	SupportTermsVersion string             `json:"support_terms_version"`
	Offer               PublicOffer        `json:"offer"`
}
type PublicCatalog struct {
	Schema      string       `json:"schema"`
	Revision    string       `json:"revision"`
	Product     string       `json:"product"`
	Environment string       `json:"environment"`
	Plans       []PublicPlan `json:"plans"`
}
type CatalogPreview struct {
	Catalog PublicCatalog `json:"catalog"`
	SHA256  string        `json:"sha256"`
}

func BuildPublicCatalog(id string, request CatalogRequest, plans []PlanSnapshot) (CatalogPreview, error) {
	if err := request.Validate(); err != nil {
		return CatalogPreview{}, err
	}
	if !catalogRevision.MatchString(id) || plans == nil || len(plans) != len(request.Plans) {
		return CatalogPreview{}, ErrInvalidCatalog
	}
	result := PublicCatalog{Schema: PublicCatalogSchema, Revision: id, Product: productcatalog.Product, Environment: request.Environment, Plans: []PublicPlan{}}
	for i, source := range plans {
		selected := request.Plans[i]
		if err := source.Validate(); err != nil {
			return CatalogPreview{}, err
		}
		frozen, err := FreezePlan(source.PlanID, source.Version, source.Definition)
		if err != nil {
			return CatalogPreview{}, err
		}
		if selected.PlanID != source.PlanID || selected.Version != source.Version || selected.ExpectedSHA256 != frozen.Digest() {
			return CatalogPreview{}, ErrCatalogPreviewConflict
		}
		// Work from the normalized, owned copy, never caller-owned nested slices.
		normalized, err := frozen.Snapshot()
		if err != nil {
			return CatalogPreview{}, err
		}
		d := normalized.Definition
		entitlements := PublicEntitlements{CatalogVersion: d.Entitlements.CatalogVersion, Features: slices.Clone(d.Entitlements.Features), FeatureSets: slices.Clone(d.Entitlements.FeatureSets), Quotas: []PublicQuota{}}
		for _, q := range d.Entitlements.Quotas {
			limit := PublicQuotaLimit{Mode: q.Limit.Mode}
			if q.Limit.Value != nil {
				value := *q.Limit.Value
				limit.Value = &value
			}
			entitlements.Quotas = append(entitlements.Quotas, PublicQuota{ID: q.ID, Limit: limit})
		}
		offer := PublicOffer{Kind: d.Offer.Kind}
		switch d.Offer.Kind {
		case "annual":
			offer.Kind = "fixed_price"
			offer.Currency, offer.AnnualAmountMinor, offer.TaxMode = d.Offer.Currency, d.Offer.AnnualAmountMinor, d.Offer.TaxMode
			offer.TermRule, offer.TermTimezone = d.Offer.TermRule, d.Offer.TermTimezone
			for _, term := range d.Offer.Terms {
				amount, err := CalculateAmount(d.Offer.AnnualAmountMinor, term)
				if err != nil {
					return CatalogPreview{}, err
				}
				offer.Terms = append(offer.Terms, PublicTerm{Years: term.Years, DiscountBasisPoints: term.DiscountBasisPoints, TotalAmountMinor: amount})
			}
		case "free":
			offer.Expiry = &PublicExpiry{Mode: string(d.Offer.Expiry.Mode), ExpiresAt: d.Offer.Expiry.ExpiresAt}
		}
		result.Plans = append(result.Plans, PublicPlan{PlanID: source.PlanID, Version: source.Version, Name: d.Name, Description: d.Description, Edition: d.Edition, Entitlements: entitlements, QuotaPolicyVersion: d.QuotaPolicyVersion, MinimumVersion: d.MinimumVersion, SupportTermsVersion: d.SupportTermsVersion, Offer: offer})
	}
	raw, err := canonical(result)
	if err != nil {
		return CatalogPreview{}, err
	}
	return CatalogPreview{Catalog: result, SHA256: digest(raw)}, nil
}

type CatalogApprovalSnapshot struct {
	Schema       string         `json:"schema"`
	ID           string         `json:"id"`
	Request      CatalogRequest `json:"request"`
	Plans        []PlanSnapshot `json:"plans"`
	PublicSHA256 string         `json:"public_sha256"`
	ApprovedBy   string         `json:"approved_by"`
	ApprovedAt   string         `json:"approved_at"`
}

func (v *CatalogApprovalSnapshot) UnmarshalJSON(data []byte) error {
	type wire CatalogApprovalSnapshot
	var result wire
	if err := strictjson.Object(data, &result, "schema", "id", "request", "plans", "public_sha256", "approved_by", "approved_at"); err != nil {
		return err
	}
	*v = CatalogApprovalSnapshot(result)
	return nil
}
func (v CatalogApprovalSnapshot) Preview() (CatalogPreview, error) {
	if v.Schema != CatalogApprovalSchema || !identifier.MatchString(v.ApprovedBy) || len(v.ApprovedBy) > 64 {
		return CatalogPreview{}, ErrInvalidCatalog
	}
	if _, err := parseTime(v.ApprovedAt); err != nil {
		return CatalogPreview{}, err
	}
	p, err := BuildPublicCatalog(v.ID, v.Request, v.Plans)
	if err != nil {
		return CatalogPreview{}, err
	}
	if p.SHA256 != v.PublicSHA256 {
		return CatalogPreview{}, ErrInvalidCatalog
	}
	return p, nil
}
func (v CatalogApprovalSnapshot) Bytes() ([]byte, error) {
	if _, err := v.Preview(); err != nil {
		return nil, err
	}
	raw, err := canonical(v)
	if err != nil {
		return nil, err
	}
	if len(raw) > 1<<20 {
		return nil, ErrInvalidCatalog
	}
	return raw, nil
}
func ParseCatalogApproval(data []byte, expected string) (CatalogApprovalSnapshot, error) {
	var result CatalogApprovalSnapshot
	if len(data) > 2<<20 || digest(data) != expected {
		return result, ErrInvalidCatalog
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return result, err
	}
	raw, err := result.Bytes()
	if err != nil {
		return result, err
	}
	if !bytes.Equal(raw, data) {
		return CatalogApprovalSnapshot{}, ErrInvalidCatalog
	}
	return result, nil
}

type CatalogApprovalRecord struct {
	Snapshot CatalogApprovalSnapshot `json:"snapshot"`
	SHA256   string                  `json:"sha256"`
	Public   CatalogPreview          `json:"public"`
	Status   string                  `json:"status"`
}

func (v CatalogApprovalRecord) PublicBytes() ([]byte, error) {
	preview, err := v.Snapshot.Preview()
	if err != nil {
		return nil, err
	}
	snapshot, err := v.Snapshot.Bytes()
	if err != nil || digest(snapshot) != v.SHA256 {
		return nil, ErrInvalidCatalog
	}
	raw, err := canonical(v.Public.Catalog)
	if err != nil || v.Public.SHA256 != preview.SHA256 || digest(raw) != preview.SHA256 {
		return nil, ErrInvalidCatalog
	}
	if v.Status != "approved" && v.Status != "exported" {
		return nil, ErrInvalidCatalog
	}
	return raw, nil
}

type ApproveCatalogInput struct {
	Request              CatalogRequest `json:"request"`
	ExpectedPublicSHA256 string         `json:"expected_public_sha256"`
}

func (v *ApproveCatalogInput) UnmarshalJSON(data []byte) error {
	type wire ApproveCatalogInput
	var result wire
	if err := strictjson.Object(data, &result, "request", "expected_public_sha256"); err != nil {
		return err
	}
	*v = ApproveCatalogInput(result)
	return v.Validate()
}
func (v ApproveCatalogInput) Validate() error {
	if !catalogDigest.MatchString(v.ExpectedPublicSHA256) {
		return ErrInvalidCatalog
	}
	return v.Request.Validate()
}
