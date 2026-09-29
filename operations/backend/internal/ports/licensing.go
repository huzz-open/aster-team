package ports

import (
	"context"
	"encoding/json"
	"time"

	"aster.local/team/operations/backend/internal/licenseprotocol"
)

type LicenseLimits struct {
	MemberSeats            int `json:"member_seats"`
	SeatOverLimitGraceDays int `json:"seat_over_limit_grace_days"`
}

type LicensePolicy struct {
	OperationID    string        `json:"operation_id"`
	LicenseID      string        `json:"license_id"`
	CustomerRef    string        `json:"customer_ref"`
	SourceType     string        `json:"source_type"`
	SourceID       string        `json:"source_id"`
	Product        string        `json:"product"`
	Edition        string        `json:"edition"`
	Features       []string      `json:"features"`
	Limits         LicenseLimits `json:"limits"`
	MinimumVersion string        `json:"minimum_version"`
	ValidUntil     time.Time     `json:"valid_until"`
	TransferLimit  int           `json:"transfer_limit"`
}

type LicenseRecord struct {
	LicensePolicy
	Status        string    `json:"status"`
	TransferCount int       `json:"transfer_count"`
	CreatedAt     time.Time `json:"created_at"`
	UpdatedAt     time.Time `json:"updated_at"`
}

type LicenseIssuance struct {
	ID                       string          `json:"id"`
	LicenseID                string          `json:"license_id"`
	RequestID                string          `json:"request_id"`
	InstallationID           string          `json:"installation_id"`
	MachineFingerprintSHA256 string          `json:"machine_fingerprint_sha256"`
	TransferSequence         int             `json:"transfer_sequence"`
	Document                 json.RawMessage `json:"document"`
	SHA256                   string          `json:"sha256"`
	IssuedAt                 time.Time       `json:"issued_at"`
	ExpiresAt                time.Time       `json:"expires_at"`
}

type LicenseSigner interface {
	Sign(context.Context, licenseprotocol.License) (licenseprotocol.License, error)
}

type LicenseVerifier interface {
	Verify([]byte) (licenseprotocol.License, error)
}
