package application

import (
	"errors"

	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
)

var ErrV2VerifierUnavailable = errors.New("trusted v2 public key unavailable")

func WithV2LicenseVerifiers(verifiers map[string]ports.LicenseVerifierV2) Option {
	return func(s *Service) {
		s.licenseVerifiersV2 = make(map[string]ports.LicenseVerifierV2, len(verifiers))
		for id, verifier := range verifiers {
			s.licenseVerifiersV2[id] = verifier
		}
	}
}
func (s *Service) verifyV2Document(document licenseprotocol.DocumentV2) error {
	verifier := s.licenseVerifiersV2[document.Claims.KeyID]
	if verifier == nil {
		return ErrV2VerifierUnavailable
	}
	return verifier.Verify(document)
}
