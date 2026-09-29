package licenseprotocol

import (
	"bytes"
	"crypto/ed25519"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"sort"
	"strconv"
	"time"
	"unicode/utf16"
)

type Limits struct {
	MemberSeats            int64 `json:"member_seats"`
	SeatOverLimitGraceDays int64 `json:"seat_over_limit_grace_days"`
}

type License struct {
	Schema                   string   `json:"schema"`
	KeyID                    string   `json:"key_id"`
	LicenseID                string   `json:"license_id"`
	Serial                   string   `json:"serial"`
	RequestID                string   `json:"request_id"`
	CustomerRef              string   `json:"customer_ref"`
	Product                  string   `json:"product"`
	Edition                  string   `json:"edition"`
	Features                 []string `json:"features"`
	Limits                   Limits   `json:"limits"`
	MinimumVersion           string   `json:"minimum_version"`
	InstallationID           string   `json:"installation_id"`
	MachineFingerprintSHA256 string   `json:"machine_fingerprint_sha256"`
	TransferSequence         int64    `json:"transfer_sequence"`
	IssuedAt                 string   `json:"issued_at"`
	NotBefore                string   `json:"not_before"`
	ExpiresAt                string   `json:"expires_at"`
	Signature                string   `json:"signature"`
}

var identifierPattern = regexp.MustCompile(`^[A-Za-z0-9._:@+/-]+$`)
var digestPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{43}$`)

func Sign(claims License, privateKey ed25519.PrivateKey) (License, error) {
	claims.Signature = ""
	if err := validate(claims); err != nil {
		return License{}, err
	}
	raw, err := claimsMap(claims)
	if err != nil {
		return License{}, err
	}
	payload, err := Canonicalize(raw)
	if err != nil {
		return License{}, err
	}
	claims.Signature = base64.RawURLEncoding.EncodeToString(ed25519.Sign(privateKey, payload))
	return claims, nil
}

func Verify(data []byte, trustedPublicKeys map[string]string) (License, error) {
	license, raw, err := parse(data)
	if err != nil {
		return License{}, err
	}
	signature, ok := raw["signature"].(string)
	if !ok {
		return License{}, errors.New("license signature is missing")
	}
	delete(raw, "signature")
	payload, err := Canonicalize(raw)
	if err != nil {
		return License{}, err
	}
	publicKeySPKI, trusted := trustedPublicKeys[license.KeyID]
	if !trusted {
		return License{}, errors.New("license key ID is not trusted")
	}
	publicKey, err := parsePublicKey(publicKeySPKI)
	if err != nil {
		return License{}, fmt.Errorf("parse license public key: %w", err)
	}
	decoded, err := base64.RawURLEncoding.DecodeString(signature)
	if err != nil || !ed25519.Verify(publicKey, payload, decoded) {
		return License{}, errors.New("license signature is invalid")
	}
	return license, nil
}

// Parse validates the exact v1 document shape and claims without trusting its
// signature. Callers that establish authenticity must use Verify.
func Parse(data []byte) (License, error) {
	license, _, err := parse(data)
	return license, err
}

func parse(data []byte) (License, map[string]any, error) {
	var raw map[string]any
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err := decoder.Decode(&raw); err != nil {
		return License{}, nil, fmt.Errorf("decode license: %w", err)
	}
	if decoder.Decode(&struct{}{}) == nil {
		return License{}, nil, errors.New("license contains trailing JSON")
	}
	if len(raw) != 18 {
		return License{}, nil, errors.New("license fields are invalid")
	}
	var license License
	typedDecoder := json.NewDecoder(bytes.NewReader(data))
	typedDecoder.DisallowUnknownFields()
	if err := typedDecoder.Decode(&license); err != nil {
		return License{}, nil, fmt.Errorf("decode typed license: %w", err)
	}
	if err := validate(license); err != nil {
		return License{}, nil, err
	}
	return license, raw, nil
}

func validate(license License) error {
	if license.Schema != "aster.license.v1" || license.Product != "aster-team" {
		return errors.New("license identity is invalid")
	}
	for name, value := range map[string]string{
		"key_id":     license.KeyID,
		"license_id": license.LicenseID, "serial": license.Serial, "request_id": license.RequestID,
		"customer_ref": license.CustomerRef, "edition": license.Edition, "minimum_version": license.MinimumVersion,
		"installation_id": license.InstallationID,
	} {
		if len(value) < 1 || len(value) > 128 || !identifierPattern.MatchString(value) {
			return fmt.Errorf("%s is invalid", name)
		}
	}
	if !digestPattern.MatchString(license.MachineFingerprintSHA256) {
		return errors.New("machine_fingerprint_sha256 is invalid")
	}
	if license.TransferSequence < 0 || license.TransferSequence > 10_000 {
		return errors.New("transfer_sequence is invalid")
	}
	if len(license.Features) == 0 || len(license.Features) > 64 {
		return errors.New("features are invalid")
	}
	seen := map[string]bool{}
	for _, feature := range license.Features {
		if len(feature) < 1 || len(feature) > 64 || !identifierPattern.MatchString(feature) || seen[feature] {
			return errors.New("feature is invalid or duplicated")
		}
		seen[feature] = true
	}
	if license.Limits.MemberSeats < 0 || license.Limits.MemberSeats > 1_000_000 ||
		license.Limits.SeatOverLimitGraceDays < 0 || license.Limits.SeatOverLimitGraceDays > 90 {
		return errors.New("license limits are invalid")
	}
	issuedAt, issuedErr := exactTime(license.IssuedAt)
	notBefore, beforeErr := exactTime(license.NotBefore)
	expiresAt, expiresErr := exactTime(license.ExpiresAt)
	if issuedErr != nil || beforeErr != nil || expiresErr != nil || issuedAt.After(notBefore) || !notBefore.Before(expiresAt) {
		return errors.New("license time range is invalid")
	}
	if license.Signature != "" {
		decoded, err := base64.RawURLEncoding.DecodeString(license.Signature)
		if err != nil || len(decoded) != ed25519.SignatureSize {
			return errors.New("license signature encoding is invalid")
		}
	}
	return nil
}

func exactTime(value string) (time.Time, error) {
	parsed, err := time.Parse(time.RFC3339Nano, value)
	if err != nil || parsed.UTC().Format("2006-01-02T15:04:05.000Z") != value {
		return time.Time{}, errors.New("time is invalid")
	}
	return parsed, nil
}

func claimsMap(license License) (map[string]any, error) {
	data, err := json.Marshal(license)
	if err != nil {
		return nil, err
	}
	var raw map[string]any
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err = decoder.Decode(&raw); err != nil {
		return nil, err
	}
	delete(raw, "signature")
	return raw, nil
}

func parsePublicKey(value string) (ed25519.PublicKey, error) {
	encoded, err := base64.RawURLEncoding.DecodeString(value)
	if err != nil {
		return nil, err
	}
	parsed, err := x509.ParsePKIXPublicKey(encoded)
	if err != nil {
		return nil, err
	}
	key, ok := parsed.(ed25519.PublicKey)
	if !ok {
		return nil, errors.New("key is not Ed25519")
	}
	return key, nil
}

func Canonicalize(value any) ([]byte, error) {
	var buffer bytes.Buffer
	if err := appendCanonical(&buffer, value); err != nil {
		return nil, err
	}
	return buffer.Bytes(), nil
}

func appendCanonical(buffer *bytes.Buffer, value any) error {
	switch typed := value.(type) {
	case nil:
		buffer.WriteString("null")
	case bool:
		buffer.WriteString(strconv.FormatBool(typed))
	case string:
		encoded, _ := json.Marshal(typed)
		buffer.Write(encoded)
	case json.Number:
		if _, err := typed.Int64(); err != nil {
			return fmt.Errorf("canonical numbers must be signed 64-bit integers: %s", typed)
		}
		buffer.WriteString(typed.String())
	case float64:
		if typed != float64(int64(typed)) {
			return errors.New("canonical numbers must be integers")
		}
		buffer.WriteString(strconv.FormatInt(int64(typed), 10))
	case []any:
		buffer.WriteByte('[')
		for index, item := range typed {
			if index > 0 {
				buffer.WriteByte(',')
			}
			if err := appendCanonical(buffer, item); err != nil {
				return err
			}
		}
		buffer.WriteByte(']')
	case map[string]any:
		keys := make([]string, 0, len(typed))
		for key := range typed {
			keys = append(keys, key)
		}
		sort.Slice(keys, func(i, j int) bool { return lessUTF16(keys[i], keys[j]) })
		buffer.WriteByte('{')
		for index, key := range keys {
			if index > 0 {
				buffer.WriteByte(',')
			}
			encoded, _ := json.Marshal(key)
			buffer.Write(encoded)
			buffer.WriteByte(':')
			if err := appendCanonical(buffer, typed[key]); err != nil {
				return err
			}
		}
		buffer.WriteByte('}')
	default:
		return fmt.Errorf("unsupported canonical JSON type %T", value)
	}
	return nil
}

func lessUTF16(left, right string) bool {
	l := utf16.Encode([]rune(left))
	r := utf16.Encode([]rune(right))
	for index := 0; index < len(l) && index < len(r); index++ {
		if l[index] != r[index] {
			return l[index] < r[index]
		}
	}
	return len(l) < len(r)
}
