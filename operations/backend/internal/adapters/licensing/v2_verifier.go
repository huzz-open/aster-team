package licensing

import (
	"bytes"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"sort"

	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
	"aster.local/team/operations/backend/internal/strictjson"
)

type V2Verifier struct {
	profile ports.IssuerProfileV2
	keys    licenseprotocol.TrustedKeysV2
}

func NewV2Verifier(profile ports.IssuerProfileV2) (*V2Verifier, error) {
	var keys licenseprotocol.TrustedKeysV2
	if err := keys.Insert(profile.KeyID, profile.PublicKeySPKI, profile.Policy); err != nil {
		return nil, err
	}
	profile.Policy = profile.Policy.Clone()
	return &V2Verifier{profile: profile, keys: keys}, nil
}
func (v *V2Verifier) Profile() ports.IssuerProfileV2 {
	profile := v.profile
	profile.Policy = profile.Policy.Clone()
	return profile
}
func (v *V2Verifier) Verify(document licenseprotocol.DocumentV2) error {
	raw, err := json.Marshal(document)
	if err != nil {
		return err
	}
	_, err = licenseprotocol.VerifyV2(raw, &v.keys)
	return err
}

type configuredVerifierV2 ports.IssuerProfileV2

func (v *configuredVerifierV2) UnmarshalJSON(data []byte) error {
	type wire configuredVerifierV2
	var result wire
	if err := strictjson.Object(data, &result, "key_id", "public_key_spki", "policy"); err != nil {
		return err
	}
	*v = configuredVerifierV2(result)
	return nil
}

// Retained trusted public keys permit recovery after private keys are removed.
// Sources here are server configuration, never a key copied from a stored record.
// Repeated exact profiles across the signing/verification registries are allowed;
// duplicate entries, conflicting scope and public-key aliases are rejected.
func LoadV2Verifiers(raw string, signers map[string]ports.LicenseSignerV2, legacy *Signer) (map[string]ports.LicenseVerifierV2, error) {
	result := map[string]ports.LicenseVerifierV2{}
	var trusted licenseprotocol.TrustedKeysV2
	var legacyPublic string
	if legacy != nil {
		der, err := x509.MarshalPKIXPublicKey(legacy.privateKey.Public())
		if err != nil {
			return nil, err
		}
		legacyPublic = base64.RawURLEncoding.EncodeToString(der)
	}
	add := func(profile ports.IssuerProfileV2) error {
		if legacy != nil && (profile.KeyID == legacy.keyID || profile.PublicKeySPKI == legacyPublic) {
			return errors.New("v2 verification keys must be separate from legacy keys")
		}
		if old := result[profile.KeyID]; old != nil {
			before, _ := json.Marshal(old.Profile())
			after, _ := json.Marshal(profile)
			if !bytes.Equal(before, after) {
				return errors.New("conflicting v2 signing and verification profiles")
			}
			return nil
		}
		if err := trusted.Insert(profile.KeyID, profile.PublicKeySPKI, profile.Policy); err != nil {
			return err
		}
		verifier, err := NewV2Verifier(profile)
		if err != nil {
			return err
		}
		result[profile.KeyID] = verifier
		return nil
	}
	ids := make([]string, 0, len(signers))
	for id := range signers {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	for _, id := range ids {
		if signers[id] == nil || signers[id].Profile().KeyID != id {
			return nil, errors.New("invalid v2 signer registry")
		}
		if err := add(signers[id].Profile()); err != nil {
			return nil, err
		}
	}
	if raw == "" {
		return result, nil
	}
	var configs []configuredVerifierV2
	if len(raw) > 1<<20 || json.Unmarshal([]byte(raw), &configs) != nil || len(configs) == 0 || len(configs) > 64 {
		return nil, errors.New("invalid v2 verification configuration")
	}
	seen := map[string]bool{}
	for index, cfg := range configs {
		if seen[cfg.KeyID] {
			return nil, fmt.Errorf("duplicate v2 verification key at index %d", index)
		}
		seen[cfg.KeyID] = true
		if err := add(ports.IssuerProfileV2(cfg)); err != nil {
			return nil, fmt.Errorf("invalid v2 verification profile at index %d: %w", index, err)
		}
	}
	return result, nil
}
