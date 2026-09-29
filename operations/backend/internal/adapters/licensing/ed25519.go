package licensing

import (
	"context"
	"crypto/ed25519"
	"crypto/x509"
	"encoding/base64"
	"errors"

	"aster.local/team/operations/backend/internal/licenseprotocol"
)

type Signer struct {
	keyID      string
	privateKey ed25519.PrivateKey
}

func New(keyID, privateKeyPKCS8 string) (*Signer, error) {
	if len(keyID) < 3 || len(keyID) > 128 {
		return nil, errors.New("license signing key ID is invalid")
	}
	for _, value := range keyID {
		if (value < 'a' || value > 'z') && (value < 'A' || value > 'Z') && (value < '0' || value > '9') && value != '_' && value != '-' && value != '.' && value != ':' {
			return nil, errors.New("license signing key ID is invalid")
		}
	}
	encoded, err := base64.RawURLEncoding.DecodeString(privateKeyPKCS8)
	if err != nil {
		return nil, errors.New("license signing private key encoding is invalid")
	}
	parsed, err := x509.ParsePKCS8PrivateKey(encoded)
	if err != nil {
		return nil, errors.New("license signing private key is invalid")
	}
	privateKey, ok := parsed.(ed25519.PrivateKey)
	if !ok {
		return nil, errors.New("license signing key must be Ed25519")
	}
	return &Signer{keyID: keyID, privateKey: privateKey}, nil
}

func (signer *Signer) Sign(_ context.Context, claims licenseprotocol.License) (licenseprotocol.License, error) {
	if claims.KeyID != "" && claims.KeyID != signer.keyID {
		return licenseprotocol.License{}, errors.New("license claims use another signing key ID")
	}
	claims.KeyID = signer.keyID
	return licenseprotocol.Sign(claims, signer.privateKey)
}

func (signer *Signer) Verify(document []byte) (licenseprotocol.License, error) {
	public, err := x509.MarshalPKIXPublicKey(signer.privateKey.Public())
	if err != nil {
		return licenseprotocol.License{}, err
	}
	return licenseprotocol.Verify(document, map[string]string{
		signer.keyID: base64.RawURLEncoding.EncodeToString(public),
	})
}
