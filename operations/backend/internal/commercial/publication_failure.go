package commercial

import "aster.local/team/operations/backend/internal/strictjson"

// Failure events describe an unsuccessful attempt, not the publication's current
// state. A commit response can be lost after success; such attempts are explicitly
// recorded as unconfirmed and must be resolved by reading the original record.
type PublicationFailure struct {
	ID                string `json:"id"`
	PublicationID     string `json:"publication_id"`
	PublicationSHA256 string `json:"publication_sha256"`
	Stage             string `json:"stage"`
	Code              string `json:"code"`
	OperatorID        string `json:"operator_id"`
	CreatedAt         string `json:"created_at"`
}

func (v *PublicationFailure) UnmarshalJSON(data []byte) error {
	type wire PublicationFailure
	var result wire
	if err := strictjson.Object(data, &result, "id", "publication_id", "publication_sha256", "stage", "code", "operator_id", "created_at"); err != nil {
		return err
	}
	*v = PublicationFailure(result)
	return v.Validate()
}
func (v PublicationFailure) Validate() error {
	for _, id := range []string{v.ID, v.PublicationID, v.OperatorID} {
		if !identifier.MatchString(id) || len(id) > 64 {
			return ErrInvalidPublication
		}
	}
	if !catalogDigest.MatchString(v.PublicationSHA256) {
		return ErrInvalidPublication
	}
	if _, err := parseTime(v.CreatedAt); err != nil {
		return ErrInvalidPublication
	}
	switch v.Stage {
	case "verification":
		if v.Code == "target_not_configured" || v.Code == "content_unverified" || v.Code == "invalid_evidence" {
			return nil
		}
	case "acceptance":
		if v.Code == "head_conflict" || v.Code == "deadline_expired" || v.Code == "commit_unconfirmed" {
			return nil
		}
	}
	return ErrInvalidPublication
}
func (v PublicationFailure) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	return canonical(v)
}
