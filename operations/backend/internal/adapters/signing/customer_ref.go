package signing

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"errors"
)

type HMACCustomerReferenceSource struct {
	key []byte
}

func NewHMACCustomerReferenceSource(encodedSecret string) (*HMACCustomerReferenceSource, error) {
	key, err := base64.RawURLEncoding.DecodeString(encodedSecret)
	if err != nil || len(key) < 32 {
		return nil, errors.New("customer reference secret must be at least 32 base64url-encoded bytes")
	}
	return &HMACCustomerReferenceSource{key: key}, nil
}

func (source *HMACCustomerReferenceSource) Reference(customerID string) (string, error) {
	if customerID == "" {
		return "", errors.New("customer ID is required")
	}
	mac := hmac.New(sha256.New, source.key)
	_, _ = mac.Write([]byte("aster-team:customer-ref:v1\x00" + customerID))
	return "customer_" + base64.RawURLEncoding.EncodeToString(mac.Sum(nil)), nil
}
