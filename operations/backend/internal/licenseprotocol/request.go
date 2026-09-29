package licenseprotocol

import (
	"errors"
	"fmt"
	"regexp"
	"time"

	"aster.local/team/operations/backend/internal/strictjson"
)

type MachineFactor struct {
	Kind   string `json:"kind"`
	SHA256 string `json:"sha256"`
}

type Request struct {
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
}

var requestIDPattern = regexp.MustCompile(`^[A-Za-z0-9._:@+/-]{8,128}$`)

func ParseRequest(data []byte) (Request, error) {
	type wire Request
	var decoded wire
	if err := strictjson.Object(data, &decoded, "schema", "request_id", "product", "product_version", "platform", "architecture", "installation_id", "machine_fingerprint_sha256", "machine_factors", "generated_at"); err != nil {
		return Request{}, fmt.Errorf("decode license request: %w", err)
	}
	request := Request(decoded)
	if request.Schema != "aster.license-request.v1" || request.Product != "aster-team" ||
		(request.Platform != "linux" && request.Platform != "windows" && request.Platform != "macos") ||
		(request.Architecture != "amd64" && request.Architecture != "arm64") || !requestIDPattern.MatchString(request.RequestID) ||
		len(request.ProductVersion) < 1 || len(request.ProductVersion) > 64 || !identifierPattern.MatchString(request.ProductVersion) ||
		!requestIDPattern.MatchString(request.InstallationID) ||
		!digestPattern.MatchString(request.MachineFingerprintSHA256) || len(request.MachineFactors) != 2 {
		return Request{}, errors.New("license request is invalid")
	}
	kinds := map[string]bool{}
	for _, factor := range request.MachineFactors {
		if (factor.Kind != "dmi_product_uuid" && factor.Kind != "machine_id") || kinds[factor.Kind] || !digestPattern.MatchString(factor.SHA256) {
			return Request{}, errors.New("license request machine factors are invalid")
		}
		kinds[factor.Kind] = true
	}
	if !kinds["dmi_product_uuid"] || !kinds["machine_id"] {
		return Request{}, errors.New("license request machine factors are incomplete")
	}
	parsed, err := time.Parse(time.RFC3339Nano, request.GeneratedAt)
	if err != nil || parsed.UTC().Format("2006-01-02T15:04:05.000Z") != request.GeneratedAt {
		return Request{}, errors.New("license request generated_at is invalid")
	}
	return request, nil
}

// Keep direct JSON unmarshalling as strict as the standalone request parser.
func (v *Request) UnmarshalJSON(data []byte) error {
	parsed, err := ParseRequest(data)
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}
func (v *MachineFactor) UnmarshalJSON(data []byte) error {
	type wire MachineFactor
	var result wire
	if err := strictjson.Object(data, &result, "kind", "sha256"); err != nil {
		return err
	}
	*v = MachineFactor(result)
	return nil
}
