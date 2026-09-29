package licenseprotocol

import (
	"bytes"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"

	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/productcatalog"
	"aster.local/team/operations/backend/internal/strictjson"
)

const SchemaV2 = "aster.license.v2"
const QuotaPolicyVersionV2 uint32 = 1

type SourceKindV2 string

const (
	FreeDistributionV2 SourceKindV2 = "free_distribution"
	CommercialOrderV2  SourceKindV2 = "commercial_order"
	ApprovedTrialV2    SourceKindV2 = "approved_trial"
)

type BindingKindV2 string

const (
	UnboundV2      BindingKindV2 = "unbound"
	InstallationV2 BindingKindV2 = "installation"
)

type ExpiryKindV2 string

const (
	FixedExpiryV2 ExpiryKindV2 = "fixed"
	NoExpiryV2    ExpiryKindV2 = "none"
)

type SourceV2 struct {
	Kind           SourceKindV2 `json:"kind"`
	DistributionID string       `json:"distribution_id,omitempty"`
	OrderID        string       `json:"order_id,omitempty"`
	TrialID        string       `json:"trial_id,omitempty"`
	CustomerRef    string       `json:"customer_ref,omitempty"`
	RequestID      string       `json:"request_id,omitempty"`
}

func (value *SourceV2) UnmarshalJSON(data []byte) error {
	var tag struct {
		Kind SourceKindV2 `json:"kind"`
	}
	if err := json.Unmarshal(data, &tag); err != nil {
		return err
	}
	fields := []string{"kind"}
	switch tag.Kind {
	case FreeDistributionV2:
		fields = append(fields, "distribution_id")
	case CommercialOrderV2:
		fields = append(fields, "order_id", "customer_ref", "request_id")
	case ApprovedTrialV2:
		fields = append(fields, "trial_id", "customer_ref", "request_id")
	default:
		return errors.New("unknown license source")
	}
	type wire SourceV2
	var result wire
	if err := strictjson.Object(data, &result, fields...); err != nil {
		return err
	}
	*value = SourceV2(result)
	return nil
}

func (value SourceV2) validate() error {
	identifiers := []string{}
	switch value.Kind {
	case FreeDistributionV2:
		if value.OrderID != "" || value.TrialID != "" || value.CustomerRef != "" || value.RequestID != "" {
			return errors.New("invalid free source fields")
		}
		identifiers = append(identifiers, value.DistributionID)
	case CommercialOrderV2:
		if value.DistributionID != "" || value.TrialID != "" {
			return errors.New("invalid order source fields")
		}
		identifiers = append(identifiers, value.OrderID, value.CustomerRef, value.RequestID)
	case ApprovedTrialV2:
		if value.DistributionID != "" || value.OrderID != "" {
			return errors.New("invalid trial source fields")
		}
		identifiers = append(identifiers, value.TrialID, value.CustomerRef, value.RequestID)
	default:
		return errors.New("unknown license source")
	}
	for _, value := range identifiers {
		if !validV2Identifier(value, 1) {
			return errors.New("invalid license source identifier")
		}
	}
	return nil
}

type BindingV2 struct {
	Mode                     BindingKindV2 `json:"mode"`
	InstallationID           string        `json:"installation_id,omitempty"`
	MachineFingerprintSHA256 string        `json:"machine_fingerprint_sha256,omitempty"`
	TransferSequence         *uint32       `json:"transfer_sequence,omitempty"`
}

func (value *BindingV2) UnmarshalJSON(data []byte) error {
	var tag struct {
		Mode BindingKindV2 `json:"mode"`
	}
	if err := json.Unmarshal(data, &tag); err != nil {
		return err
	}
	fields := []string{"mode"}
	switch tag.Mode {
	case UnboundV2:
	case InstallationV2:
		fields = append(fields, "installation_id", "machine_fingerprint_sha256", "transfer_sequence")
	default:
		return errors.New("unknown binding mode")
	}
	type wire BindingV2
	var result wire
	if err := strictjson.Object(data, &result, fields...); err != nil {
		return err
	}
	*value = BindingV2(result)
	return nil
}
func (value BindingV2) validate() error {
	switch value.Mode {
	case UnboundV2:
		if value.InstallationID != "" || value.MachineFingerprintSHA256 != "" || value.TransferSequence != nil {
			return errors.New("unbound license contains installation fields")
		}
	case InstallationV2:
		if !validV2Identifier(value.InstallationID, 8) || !digestPattern.MatchString(value.MachineFingerprintSHA256) || value.TransferSequence == nil || *value.TransferSequence > 10000 {
			return errors.New("invalid installation binding")
		}
	default:
		return errors.New("unknown binding mode")
	}
	return nil
}

// Validate exposes binding validation to immutable commercial source
// contracts without requiring them to synthesize a complete license claim.
func (value BindingV2) Validate() error { return value.validate() }

type ExpiryV2 struct {
	Mode      ExpiryKindV2 `json:"mode"`
	ExpiresAt string       `json:"expires_at,omitempty"`
}

func (value *ExpiryV2) UnmarshalJSON(data []byte) error {
	var tag struct {
		Mode ExpiryKindV2 `json:"mode"`
	}
	if err := json.Unmarshal(data, &tag); err != nil {
		return err
	}
	fields := []string{"mode"}
	switch tag.Mode {
	case NoExpiryV2:
	case FixedExpiryV2:
		fields = append(fields, "expires_at")
	default:
		return errors.New("unknown expiry mode")
	}
	type wire ExpiryV2
	var result wire
	if err := strictjson.Object(data, &result, fields...); err != nil {
		return err
	}
	*value = ExpiryV2(result)
	return nil
}

type ValidityV2 struct {
	NotBefore string   `json:"not_before"`
	Expiry    ExpiryV2 `json:"expiry"`
}

func (value *ValidityV2) UnmarshalJSON(data []byte) error {
	type wire ValidityV2
	var result wire
	if err := strictjson.Object(data, &result, "not_before", "expiry"); err != nil {
		return err
	}
	*value = ValidityV2(result)
	return nil
}

type ClaimsV2 struct {
	Schema             string                      `json:"schema"`
	KeyID              string                      `json:"key_id"`
	LicenseID          string                      `json:"license_id"`
	Serial             string                      `json:"serial"`
	Product            string                      `json:"product"`
	Edition            string                      `json:"edition"`
	PlanID             string                      `json:"plan_id"`
	PlanVersion        uint32                      `json:"plan_version"`
	Source             SourceV2                    `json:"source"`
	Entitlements       productcatalog.Entitlements `json:"entitlements"`
	QuotaPolicyVersion uint32                      `json:"quota_policy_version"`
	Binding            BindingV2                   `json:"binding"`
	Validity           ValidityV2                  `json:"validity"`
	IssuedAt           string                      `json:"issued_at"`
	MinimumVersion     string                      `json:"minimum_version"`
}

func (value *ClaimsV2) UnmarshalJSON(data []byte) error {
	type wire ClaimsV2
	var result wire
	if err := strictjson.Object(data, &result, "schema", "key_id", "license_id", "serial", "product", "edition", "plan_id", "plan_version", "source", "entitlements", "quota_policy_version", "binding", "validity", "issued_at", "minimum_version"); err != nil {
		return err
	}
	*value = ClaimsV2(result)
	return nil
}

func (value ClaimsV2) Validate() error {
	if value.Schema != SchemaV2 || value.Product != "aster-team" {
		return errors.New("invalid v2 license identity")
	}
	for _, id := range []string{value.KeyID, value.LicenseID, value.Serial, value.Edition, value.PlanID} {
		if !validV2Identifier(id, 1) {
			return errors.New("invalid license identifier")
		}
	}
	if value.PlanVersion == 0 || value.QuotaPolicyVersion != QuotaPolicyVersionV2 {
		return errors.New("invalid license policy or plan version")
	}
	if !ValidMinimumVersionV2(value.MinimumVersion) {
		return errors.New("invalid minimum version")
	}
	if err := value.Source.validate(); err != nil {
		return err
	}
	if err := value.Binding.validate(); err != nil {
		return err
	}
	if err := value.Entitlements.Validate(); err != nil {
		return err
	}
	if len(value.Entitlements.FeatureSets) > 0 && value.Source.Kind != CommercialOrderV2 {
		return errors.New("feature sets require commercial subscription")
	}
	if value.Source.Kind == FreeDistributionV2 {
		if value.Binding.Mode != UnboundV2 {
			return errors.New("free distribution must use unbound policy")
		}
	} else if value.Binding.Mode != InstallationV2 || value.Validity.Expiry.Mode != FixedExpiryV2 {
		return errors.New("commercial and trial licenses must be bound and time-limited")
	}
	issued, issuedErr := exactTime(value.IssuedAt)
	begins, beginsErr := exactTime(value.Validity.NotBefore)
	if issuedErr != nil || beginsErr != nil {
		return errors.New("invalid license time range")
	}
	switch value.Validity.Expiry.Mode {
	case FixedExpiryV2:
		end, err := exactTime(value.Validity.Expiry.ExpiresAt)
		if err != nil || !begins.Before(end) || !issued.Before(end) {
			return errors.New("invalid expiry")
		}
	case NoExpiryV2:
		if value.Validity.Expiry.ExpiresAt != "" {
			return errors.New("non-expiring policy cannot include an expiry date")
		}
	default:
		return errors.New("unknown expiry mode")
	}
	return nil
}

func validV2Identifier(value string, minimum int) bool {
	return len(value) >= minimum && len(value) <= 128 && identifierPattern.MatchString(value)
}
func ValidMinimumVersionV2(value string) bool {
	if !domain.ValidSemanticVersion(value) {
		return false
	}
	core := strings.SplitN(strings.SplitN(value, "+", 2)[0], "-", 2)[0]
	for _, part := range strings.Split(core, ".") {
		if _, err := strconv.ParseUint(part, 10, 64); err != nil {
			return false
		}
	}
	return true
}

type DocumentV2 struct {
	Claims    ClaimsV2 `json:"claims"`
	Signature string   `json:"signature"`
}

func (value *DocumentV2) UnmarshalJSON(data []byte) error {
	type wire DocumentV2
	var result wire
	if err := strictjson.Object(data, &result, "claims", "signature"); err != nil {
		return err
	}
	*value = DocumentV2(result)
	return nil
}

// A populated VerifiedV2 is produced by verification. Its zero value grants nothing.
// Accessors return owned copies;
// mutating a UI/API projection cannot mutate the authenticated claims.
type VerifiedV2 struct{ document DocumentV2 }

func (value VerifiedV2) Document() DocumentV2 { return cloneDocumentV2(value.document) }
func (value VerifiedV2) Claims() ClaimsV2     { return cloneDocumentV2(value.document).Claims }

// IssuerPolicyV2 is trusted configuration, not a field supplied by a License.
type IssuerPolicyV2 struct {
	Sources            []SourceKindV2              `json:"sources"`
	Bindings           []BindingKindV2             `json:"bindings"`
	Expiries           []ExpiryKindV2              `json:"expiries"`
	EntitlementCeiling productcatalog.Entitlements `json:"entitlement_ceiling"`
}

func (value *IssuerPolicyV2) UnmarshalJSON(data []byte) error {
	type wire IssuerPolicyV2
	var result wire
	if err := strictjson.Object(data, &result, "sources", "bindings", "expiries", "entitlement_ceiling"); err != nil {
		return err
	}
	*value = IssuerPolicyV2(result)
	return value.Validate()
}

func validScopeV2[T comparable](items, supported []T) bool {
	if len(items) == 0 {
		return false
	}
	seen := map[T]bool{}
	for _, item := range items {
		if !slices.Contains(supported, item) || seen[item] {
			return false
		}
		seen[item] = true
	}
	return true
}

// Clone freezes mutable slices and quota pointers at a trusted configuration boundary.
func (value IssuerPolicyV2) Clone() IssuerPolicyV2 {
	value.Sources = slices.Clone(value.Sources)
	value.Bindings = slices.Clone(value.Bindings)
	value.Expiries = slices.Clone(value.Expiries)
	value.EntitlementCeiling = cloneEntitlementsV2(value.EntitlementCeiling)
	return value
}

func (value IssuerPolicyV2) Validate() error {
	if !validScopeV2(value.Sources, []SourceKindV2{FreeDistributionV2, CommercialOrderV2, ApprovedTrialV2}) || !validScopeV2(value.Bindings, []BindingKindV2{UnboundV2, InstallationV2}) || !validScopeV2(value.Expiries, []ExpiryKindV2{FixedExpiryV2, NoExpiryV2}) {
		return errors.New("issuer policy must explicitly declare its scope")
	}
	if slices.Contains(value.Sources, FreeDistributionV2) && len(value.Sources) != 1 {
		return errors.New("free distribution signing keys must be separate from commercial and trial signing keys")
	}
	if len(value.EntitlementCeiling.FeatureSets) > 0 && !slices.Equal(value.Sources, []SourceKindV2{CommercialOrderV2}) {
		return errors.New("feature set issuer must be commercial only")
	}
	return value.EntitlementCeiling.Validate()
}
func (value IssuerPolicyV2) Authorize(claims ClaimsV2) error {
	if err := value.Validate(); err != nil {
		return err
	}
	if err := claims.Validate(); err != nil {
		return err
	}
	if !slices.Contains(value.Sources, claims.Source.Kind) || !slices.Contains(value.Bindings, claims.Binding.Mode) || !slices.Contains(value.Expiries, claims.Validity.Expiry.Mode) {
		return errors.New("license exceeds issuer policy")
	}
	return claims.Entitlements.EnsureWithin(value.EntitlementCeiling)
}

type scopedKeyV2 struct {
	key    ed25519.PublicKey
	policy IssuerPolicyV2
}

// TrustedKeysV2 deliberately exposes no legacy map or unscoped lookup.
type TrustedKeysV2 struct{ keys map[string]scopedKeyV2 }

func (keys *TrustedKeysV2) Insert(keyID, publicKeySPKI string, policy IssuerPolicyV2) error {
	if !validV2Identifier(keyID, 1) {
		return errors.New("invalid key ID")
	}
	if err := policy.Validate(); err != nil {
		return err
	}
	if _, exists := keys.keys[keyID]; exists {
		return errors.New("duplicate trusted key ID")
	}
	encodedSPKI, err := base64.RawURLEncoding.Strict().DecodeString(publicKeySPKI)
	if err != nil || base64.RawURLEncoding.EncodeToString(encodedSPKI) != publicKeySPKI {
		return errors.New("noncanonical public key encoding")
	}
	key, err := parsePublicKey(publicKeySPKI)
	if err != nil {
		return err
	}
	for _, record := range keys.keys {
		if bytes.Equal(record.key, key) {
			return errors.New("scoped signing material cannot be registered under another key ID")
		}
	}
	if keys.keys == nil {
		keys.keys = map[string]scopedKeyV2{}
	}
	keys.keys[keyID] = scopedKeyV2{key: slices.Clone(key), policy: policy.Clone()}
	return nil
}

func CanonicalClaimsV2(claims ClaimsV2) ([]byte, error) {
	data, err := json.Marshal(claims)
	if err != nil {
		return nil, err
	}
	var raw map[string]any
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err := decoder.Decode(&raw); err != nil {
		return nil, err
	}
	return Canonicalize(raw)
}
func SignV2(claims ClaimsV2, privateKey ed25519.PrivateKey) (DocumentV2, error) {
	if len(privateKey) != ed25519.PrivateKeySize {
		return DocumentV2{}, errors.New("invalid signing key")
	}
	if err := claims.Validate(); err != nil {
		return DocumentV2{}, err
	}
	payload, err := CanonicalClaimsV2(claims)
	if err != nil {
		return DocumentV2{}, err
	}
	document := DocumentV2{Claims: claims, Signature: base64.RawURLEncoding.EncodeToString(ed25519.Sign(privateKey, payload))}
	return cloneDocumentV2(document), nil
}
func VerifyV2(data []byte, keys *TrustedKeysV2) (VerifiedV2, error) {
	if len(data) > 1<<20 {
		return VerifiedV2{}, errors.New("license document is too large")
	}
	var document DocumentV2
	if err := json.Unmarshal(data, &document); err != nil {
		return VerifiedV2{}, err
	}
	if err := document.Claims.Validate(); err != nil {
		return VerifiedV2{}, err
	}
	if keys == nil {
		return VerifiedV2{}, errors.New("missing trusted keys")
	}
	record, exists := keys.keys[document.Claims.KeyID]
	if !exists {
		return VerifiedV2{}, errors.New("untrusted v2 key")
	}
	signature, err := base64.RawURLEncoding.Strict().DecodeString(document.Signature)
	if err != nil || len(signature) != ed25519.SignatureSize || base64.RawURLEncoding.EncodeToString(signature) != document.Signature {
		return VerifiedV2{}, errors.New("invalid signature encoding")
	}
	payload, err := CanonicalClaimsV2(document.Claims)
	if err != nil {
		return VerifiedV2{}, err
	}
	if !ed25519.Verify(record.key, payload, signature) {
		return VerifiedV2{}, errors.New("invalid signature")
	}
	if err := record.policy.Authorize(document.Claims); err != nil {
		return VerifiedV2{}, fmt.Errorf("issuer scope: %w", err)
	}
	return VerifiedV2{document: document}, nil
}
func cloneEntitlementsV2(value productcatalog.Entitlements) productcatalog.Entitlements {
	value.Features = slices.Clone(value.Features)
	value.FeatureSets = slices.Clone(value.FeatureSets)
	value.Quotas = slices.Clone(value.Quotas)
	for i := range value.Quotas {
		if p := value.Quotas[i].Limit.Value; p != nil {
			n := *p
			value.Quotas[i].Limit.Value = &n
		}
	}
	return value
}
func cloneDocumentV2(value DocumentV2) DocumentV2 {
	value.Claims.Entitlements = cloneEntitlementsV2(value.Claims.Entitlements)
	if p := value.Claims.Binding.TransferSequence; p != nil {
		n := *p
		value.Claims.Binding.TransferSequence = &n
	}
	return value
}
