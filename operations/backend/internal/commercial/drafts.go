package commercial

import (
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"time"

	"aster.local/team/operations/backend/internal/strictjson"
)

const PlanDraftSchema = "aster.plan-draft.v1"

// This conflict proves the operation was not saved. An ambiguous transport
// failure is not evidence that a client may discard its original operation ID.
var ErrDraftRevisionConflict = fmt.Errorf("%w: draft head changed", ErrConflict)

type PlanDraftSnapshot struct {
	Schema          string     `json:"schema"`
	DraftID         string     `json:"draft_id"`
	Revision        uint32     `json:"revision"`
	PlanID          string     `json:"plan_id"`
	ExpectedVersion uint32     `json:"expected_version"`
	Definition      Definition `json:"definition"`
}

func (v *PlanDraftSnapshot) UnmarshalJSON(data []byte) error {
	type wire PlanDraftSnapshot
	var result wire
	if err := strictjson.Object(data, &result, "schema", "draft_id", "revision", "plan_id", "expected_version", "definition"); err != nil {
		return err
	}
	*v = PlanDraftSnapshot(result)
	return nil
}

func (v PlanDraftSnapshot) Bytes() ([]byte, error) {
	if v.Schema != PlanDraftSchema || !identifier.MatchString(v.DraftID) || v.Revision == 0 || v.ExpectedVersion == math.MaxUint32 {
		return nil, errors.New("invalid plan draft identity or revision")
	}
	// A saved draft has complete, valid terms but grants no sale, publication or
	// signing authority. FreezePlan also normalizes the capability dependency closure.
	plan, err := FreezePlan(v.PlanID, v.ExpectedVersion+1, v.Definition)
	if err != nil {
		return nil, err
	}
	snapshot, err := plan.Snapshot()
	if err != nil {
		return nil, err
	}
	v.Definition = snapshot.Definition
	return canonical(v)
}

func ParsePlanDraft(raw []byte, digest string) (PlanDraftSnapshot, error) {
	var value PlanDraftSnapshot
	if ContentDigest(raw) != digest {
		return value, errors.New("draft digest does not match content")
	}
	if err := json.Unmarshal(raw, &value); err != nil {
		return value, err
	}
	normalized, err := value.Bytes()
	if err != nil {
		return value, err
	}
	if string(normalized) != string(raw) {
		return value, errors.New("draft is not canonical")
	}
	return value, nil
}

type PlanDraftRecord struct {
	Snapshot    PlanDraftSnapshot `json:"snapshot"`
	SHA256      string            `json:"sha256"`
	OperationID string            `json:"operation_id"`
	CreatedBy   string            `json:"created_by"`
	CreatedAt   time.Time         `json:"created_at"`
}
