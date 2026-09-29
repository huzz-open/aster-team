package ports

import (
	"context"

	"aster.local/team/operations/backend/internal/licenseprotocol"
)

type IssuerProfileV2 struct {
	KeyID         string                         `json:"key_id"`
	PublicKeySPKI string                         `json:"public_key_spki"`
	Policy        licenseprotocol.IssuerPolicyV2 `json:"policy"`
}

type LicenseVerifierV2 interface {
	Profile() IssuerProfileV2
	Verify(licenseprotocol.DocumentV2) error
}

type LicenseSignerV2 interface {
	LicenseVerifierV2
	Sign(context.Context, licenseprotocol.ClaimsV2) (licenseprotocol.DocumentV2, error)
}
