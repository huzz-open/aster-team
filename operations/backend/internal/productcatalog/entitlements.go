package productcatalog

import (
	"encoding/json"
	"fmt"
	"slices"

	"aster.local/team/operations/backend/internal/strictjson"
)

// Limits distinguish a finite zero from an explicitly unlimited grant.
// A missing value is never treated as unlimited.
type QuotaLimit struct {
	Mode  string  `json:"mode"`
	Value *uint32 `json:"value,omitempty"`
}

func Limited(value uint32) QuotaLimit { return QuotaLimit{Mode: "limited", Value: &value} }
func Unlimited() QuotaLimit           { return QuotaLimit{Mode: "unlimited"} }

func (limit QuotaLimit) Validate() error {
	if (limit.Mode == "limited" && limit.Value != nil) || (limit.Mode == "unlimited" && limit.Value == nil) {
		return nil
	}
	return fmt.Errorf("invalid quota limit")
}

func (limit *QuotaLimit) UnmarshalJSON(data []byte) error {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	var mode string
	if err := json.Unmarshal(raw["mode"], &mode); err != nil {
		return err
	}
	type wireLimit QuotaLimit
	var wire wireLimit
	fields := []string{"mode"}
	if mode == "limited" {
		fields = append(fields, "value")
	}
	if err := strictjson.Object(data, &wire, fields...); err != nil {
		return err
	}
	result := QuotaLimit(wire)
	if err := result.Validate(); err != nil {
		return err
	}
	*limit = result
	return nil
}

func (limit QuotaLimit) Permits(occupied uint32) bool {
	return limit.Validate() == nil && (limit.Mode == "unlimited" || occupied <= *limit.Value)
}

func (limit QuotaLimit) IsWithin(ceiling QuotaLimit) bool {
	if limit.Validate() != nil || ceiling.Validate() != nil {
		return false
	}
	return ceiling.Mode == "unlimited" || (limit.Mode == "limited" && *limit.Value <= *ceiling.Value)
}

type QuotaGrant struct {
	ID    QuotaID    `json:"id"`
	Limit QuotaLimit `json:"limit"`
}

type Entitlements struct {
	FeatureSets    []FeatureSetID `json:"feature_sets,omitempty"`
	CatalogVersion uint32         `json:"catalog_version"`
	Features       []CapabilityID `json:"features"`
	Quotas         []QuotaGrant   `json:"quotas"`
}

func (grant *QuotaGrant) UnmarshalJSON(data []byte) error {
	type wireGrant QuotaGrant
	var wire wireGrant
	if err := strictjson.Object(data, &wire, "id", "limit"); err != nil {
		return err
	}
	*grant = QuotaGrant(wire)
	return nil
}

func (value *Entitlements) UnmarshalJSON(data []byte) error {
	type wireEntitlements Entitlements
	var wire wireEntitlements
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	fields := []string{"catalog_version", "features", "quotas"}
	if _, exists := raw["feature_sets"]; exists {
		fields = append(fields, "feature_sets")
	}
	if err := strictjson.Object(data, &wire, fields...); err != nil {
		return err
	}
	result := Entitlements(wire)
	if err := result.Validate(); err != nil {
		return err
	}
	*value = result
	return nil
}

func (value Entitlements) Validate() error {
	if value.CatalogVersion != Version {
		return fmt.Errorf("unsupported capability catalog version")
	}
	if value.Features == nil {
		return fmt.Errorf("features must be explicitly declared")
	}
	features := make(map[CapabilityID]bool, len(value.Features))
	for _, id := range value.Features {
		if _, ok := FindCapability(id); !ok || features[id] {
			return fmt.Errorf("unknown or duplicate capability %q", id)
		}
		features[id] = true
	}
	sets := map[FeatureSetID]bool{}
	for _, id := range value.FeatureSets {
		if !slices.Contains(FeatureSets(), id) || sets[id] {
			return fmt.Errorf("unknown or duplicate feature set")
		}
		sets[id] = true
	}
	for _, id := range value.EffectiveFeatures() {
		features[id] = true
	}
	for _, entry := range Capabilities() {
		if features[entry.ID] {
			for _, dependency := range entry.Requires {
				if !features[dependency] {
					return fmt.Errorf("missing capability dependency %q", dependency)
				}
			}
		}
	}
	quotas := make(map[QuotaID]bool, len(value.Quotas))
	for _, grant := range value.Quotas {
		if _, ok := FindQuota(grant.ID); !ok || quotas[grant.ID] {
			return fmt.Errorf("unknown or duplicate quota %q", grant.ID)
		}
		if err := grant.Limit.Validate(); err != nil {
			return err
		}
		quotas[grant.ID] = true
	}
	if len(quotas) != len(Quotas()) {
		return fmt.Errorf("every quota must be explicitly declared exactly once")
	}
	return nil
}

// EffectiveFeatures is a projection of signed rights onto compiled metadata.
func (value Entitlements) EffectiveFeatures() []CapabilityID {
	result := []CapabilityID{}
	for _, entry := range Capabilities() {
		if slices.Contains(value.Features, entry.ID) || slices.Contains(value.FeatureSets, entry.FeatureSet) {
			result = append(result, entry.ID)
		}
	}
	return result
}

func (value Entitlements) Quota(id QuotaID) (QuotaLimit, bool) {
	for _, grant := range value.Quotas {
		if grant.ID == id {
			return grant.Limit, true
		}
	}
	return QuotaLimit{}, false
}

func (value Entitlements) EnsureWithin(ceiling Entitlements) error {
	if err := value.Validate(); err != nil {
		return err
	}
	if err := ceiling.Validate(); err != nil {
		return err
	}
	allowed := make(map[CapabilityID]bool, len(ceiling.Features))
	for _, id := range ceiling.EffectiveFeatures() {
		allowed[id] = true
	}
	for _, id := range value.FeatureSets {
		if !slices.Contains(ceiling.FeatureSets, id) {
			return fmt.Errorf("feature set exceeds issuer scope")
		}
	}
	for _, id := range value.Features {
		if !allowed[id] {
			return fmt.Errorf("capability exceeds issuer scope")
		}
	}
	for _, grant := range value.Quotas {
		maximum, exists := ceiling.Quota(grant.ID)
		if !exists || !grant.Limit.IsWithin(maximum) {
			return fmt.Errorf("quota exceeds issuer scope")
		}
	}
	return nil
}

// ParseEntitlements rejects duplicate fields and trailing input before decoding.
// Successful parsing is not a signature check or an authorization grant.
func ParseEntitlements(data []byte) (Entitlements, error) {
	var value Entitlements
	if err := json.Unmarshal(data, &value); err != nil {
		return Entitlements{}, err
	}
	return value, nil
}
