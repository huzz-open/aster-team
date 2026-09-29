package upgradetarget

import (
	"aster.local/team/operations/backend/internal/domain"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
)

type Secrets struct{ aead cipher.AEAD }

func NewSecrets(encodedKey string) (*Secrets, error) {
	key, err := base64.StdEncoding.DecodeString(encodedKey)
	if err != nil || len(key) != 32 {
		return nil, errors.New("upgrade credential key must be base64 encoded 32 bytes")
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	aead, err := cipher.NewGCM(block)
	return &Secrets{aead: aead}, err
}

func (s *Secrets) Seal(id string, value domain.UpgradeCredentials) (string, error) {
	plain, err := json.Marshal(value)
	if err != nil {
		return "", err
	}
	nonce := make([]byte, s.aead.NonceSize())
	if _, err = rand.Read(nonce); err != nil {
		return "", err
	}
	return base64.StdEncoding.EncodeToString(s.aead.Seal(nonce, nonce, plain, []byte("upgrade-environment:v1:"+id))), nil
}

func (s *Secrets) Open(id, encoded string) (domain.UpgradeCredentials, error) {
	var value domain.UpgradeCredentials
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil || len(data) < s.aead.NonceSize() {
		return value, errors.New("invalid upgrade credential envelope")
	}
	plain, err := s.aead.Open(nil, data[:s.aead.NonceSize()], data[s.aead.NonceSize():], []byte("upgrade-environment:v1:"+id))
	if err != nil {
		return value, errors.New("cannot decrypt upgrade credentials")
	}
	err = json.Unmarshal(plain, &value)
	return value, err
}
