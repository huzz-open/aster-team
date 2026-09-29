package licenseprotocol

import (
	"encoding/json"
	"errors"
	"strings"

	"aster.local/team/operations/backend/internal/productcatalog"
	"aster.local/team/operations/backend/internal/strictjson"
)

const RequestSchemaV2 = "aster.license-request.v2"

// Compatibility is an installation's declaration, never an entitlement grant
// or proof of customer identity. Only a runtime implementing the complete v2
// import/policy path may generate this request; the existing runtime emits v1.
type RequestV2 struct {
	Schema                   string          `json:"schema"`
	RequestID                string          `json:"request_id"`
	Product                  string          `json:"product"`
	ProductVersion           string          `json:"product_version"`
	Platform                 string          `json:"platform"`
	Architecture             string          `json:"architecture"`
	InstallationID           string          `json:"installation_id"`
	MachineFingerprintSHA256 string          `json:"machine_fingerprint_sha256"`
	MachineFactors           []MachineFactor `json:"machine_factors"`
	GeneratedAt              string          `json:"generated_at"`
	LicenseSchema            string          `json:"license_schema"`
	CapabilityCatalogVersion uint32          `json:"capability_catalog_version"`
	QuotaPolicyVersion       uint32          `json:"quota_policy_version"`
}

func ParseRequestV2(data []byte) (RequestV2, error) {
	type wire RequestV2
	var result wire
	if err := strictjson.Object(data, &result, "schema", "request_id", "product", "product_version", "platform", "architecture", "installation_id", "machine_fingerprint_sha256", "machine_factors", "generated_at", "license_schema", "capability_catalog_version", "quota_policy_version"); err != nil {
		return RequestV2{}, err
	}
	v := RequestV2(result)
	return v, v.Validate()
}
func (v *RequestV2) UnmarshalJSON(data []byte) error {
	parsed, err := ParseRequestV2(data)
	if err == nil {
		*v = parsed
	}
	return err
}
func (v RequestV2) Validate() error {
	if v.Schema != RequestSchemaV2 || v.LicenseSchema != SchemaV2 || v.CapabilityCatalogVersion != productcatalog.Version || v.QuotaPolicyVersion != QuotaPolicyVersionV2 || !ValidMinimumVersionV2(v.ProductVersion) {
		return errors.New("unsupported installation request protocol or product version")
	}
	// Reuse all machine, identity and exact-time checks without changing the v1
	// wire format or making a legacy request implicitly v2-compatible.
	base := Request{Schema: "aster.license-request.v1", RequestID: v.RequestID, Product: v.Product, ProductVersion: v.ProductVersion, Platform: v.Platform, Architecture: v.Architecture, InstallationID: v.InstallationID, MachineFingerprintSHA256: v.MachineFingerprintSHA256, MachineFactors: v.MachineFactors, GeneratedAt: v.GeneratedAt}
	raw, err := json.Marshal(base)
	if err != nil {
		return err
	}
	_, err = ParseRequest(raw)
	return err
}
func (v RequestV2) Supports(minimumVersion string, catalogVersion, quotaPolicyVersion uint32) bool {
	return v.Validate() == nil && v.CapabilityCatalogVersion == catalogVersion && v.QuotaPolicyVersion == quotaPolicyVersion && VersionAtLeastV2(v.ProductVersion, minimumVersion)
}

// SemVer precedence excludes build metadata. Numeric identifiers are compared
// by length then bytes, so arbitrarily large prerelease identifiers do not wrap.
func VersionAtLeastV2(actual, minimum string) bool {
	if !ValidMinimumVersionV2(actual) || !ValidMinimumVersionV2(minimum) {
		return false
	}
	a := strings.SplitN(strings.SplitN(actual, "+", 2)[0], "-", 2)
	b := strings.SplitN(strings.SplitN(minimum, "+", 2)[0], "-", 2)
	left, right := strings.Split(a[0], "."), strings.Split(b[0], ".")
	for i := range left {
		if c := compareNumericIdentifier(left[i], right[i]); c != 0 {
			return c > 0
		}
	}
	if len(a) == 1 || len(b) == 1 {
		return len(a) == 1
	}
	left, right = strings.Split(a[1], "."), strings.Split(b[1], ".")
	for i := 0; i < min(len(left), len(right)); i++ {
		l, r := left[i], right[i]
		ln, rn := numericIdentifier(l), numericIdentifier(r)
		c := strings.Compare(l, r)
		if ln && rn {
			c = compareNumericIdentifier(l, r)
		} else if ln != rn {
			return rn
		}
		if c != 0 {
			return c > 0
		}
	}
	return len(left) >= len(right)
}
func numericIdentifier(v string) bool {
	return strings.IndexFunc(v, func(r rune) bool { return r < '0' || r > '9' }) == -1
}
func compareNumericIdentifier(a, b string) int {
	if len(a) < len(b) {
		return -1
	}
	if len(a) > len(b) {
		return 1
	}
	return strings.Compare(a, b)
}
