package licensing

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"

	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
	"aster.local/team/operations/backend/internal/strictjson"
)

// V2Signer holds a fixed scope from trusted Operations configuration. It does
// not approve a plan or order; the application must supply an approved snapshot.
type V2Signer struct {
	signer *Signer
	policy licenseprotocol.IssuerPolicyV2
}

func (signer *V2Signer) Profile() ports.IssuerProfileV2 {
	public, _ := x509.MarshalPKIXPublicKey(signer.signer.privateKey.Public())
	return ports.IssuerProfileV2{KeyID: signer.signer.keyID, PublicKeySPKI: base64.RawURLEncoding.EncodeToString(public), Policy: signer.policy.Clone()}
}
func (signer *V2Signer) Verify(document licenseprotocol.DocumentV2) error {
	profile := signer.Profile()
	var keys licenseprotocol.TrustedKeysV2
	if err := keys.Insert(profile.KeyID, profile.PublicKeySPKI, profile.Policy); err != nil {
		return err
	}
	raw, err := json.Marshal(document)
	if err != nil {
		return err
	}
	_, err = licenseprotocol.VerifyV2(raw, &keys)
	return err
}

type configuredSignerV2 struct {
	KeyID           string                         `json:"key_id"`
	PrivateKeyPKCS8 string                         `json:"private_key_pkcs8"`
	Policy          licenseprotocol.IssuerPolicyV2 `json:"policy"`
}

func (v *configuredSignerV2) UnmarshalJSON(data []byte) error {
	type wire configuredSignerV2
	var r wire
	if err := strictjson.Object(data, &r, "key_id", "private_key_pkcs8", "policy"); err != nil {
		return err
	}
	*v = configuredSignerV2(r)
	return nil
}

// Only trusted server configuration supplies private keys and scopes. Empty
// configuration leaves v2 signing unavailable; it never falls back to v1.
func LoadV2Signers(raw string, legacy *Signer) (map[string]ports.LicenseSignerV2, error) {
	result := map[string]ports.LicenseSignerV2{}
	if raw == "" {
		return result, nil
	}
	var configs []configuredSignerV2
	if len(raw) > 1<<20 || json.Unmarshal([]byte(raw), &configs) != nil || len(configs) == 0 || len(configs) > 16 {
		return nil, errors.New("invalid v2 signer configuration")
	}
	var trusted licenseprotocol.TrustedKeysV2
	for index, cfg := range configs {
		signer, err := NewV2(cfg.KeyID, cfg.PrivateKeyPKCS8, cfg.Policy)
		if err != nil {
			return nil, fmt.Errorf("invalid v2 signer at index %d", index)
		}
		if legacy != nil && (cfg.KeyID == legacy.keyID || bytes.Equal(signer.signer.privateKey.Public().(ed25519.PublicKey), legacy.privateKey.Public().(ed25519.PublicKey))) {
			return nil, errors.New("v2 signing keys must be separate from legacy signing keys")
		}
		profile := signer.Profile()
		if err = trusted.Insert(profile.KeyID, profile.PublicKeySPKI, profile.Policy); err != nil {
			return nil, fmt.Errorf("conflicting v2 signer at index %d", index)
		}
		result[profile.KeyID] = signer
	}
	return result, nil
}

func NewV2(keyID, privateKeyPKCS8 string, policy licenseprotocol.IssuerPolicyV2) (*V2Signer, error) {
	if err := policy.Validate(); err != nil {
		return nil, err
	}
	signer, err := New(keyID, privateKeyPKCS8)
	if err != nil {
		return nil, err
	}
	return &V2Signer{signer: signer, policy: policy.Clone()}, nil
}

func (signer *V2Signer) Sign(ctx context.Context, claims licenseprotocol.ClaimsV2) (licenseprotocol.DocumentV2, error) {
	if err := ctx.Err(); err != nil {
		return licenseprotocol.DocumentV2{}, err
	}
	if signer == nil || signer.signer == nil {
		return licenseprotocol.DocumentV2{}, errors.New("v2 signer is not configured")
	}
	if claims.KeyID != "" && claims.KeyID != signer.signer.keyID {
		return licenseprotocol.DocumentV2{}, errors.New("license claims use another signing key ID")
	}
	claims.KeyID = signer.signer.keyID
	if err := signer.policy.Authorize(claims); err != nil {
		return licenseprotocol.DocumentV2{}, err
	}
	return licenseprotocol.SignV2(claims, signer.signer.privateKey)
}
