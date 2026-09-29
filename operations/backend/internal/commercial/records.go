package commercial

import (
	"errors"
	"fmt"
	"time"
)

var ErrConflict = errors.New("commercial snapshot version or operation conflict")

// Returned only after proving this operation has not been stored, while holding
// the transactional operation/head locks. A caller may explicitly rebase it.
var ErrPlanVersionConflict = fmt.Errorf("%w: plan head changed before operation was stored", ErrConflict)
var ErrNotFound = errors.New("commercial snapshot not found")

type PlanVersionRecord struct {
	Snapshot    PlanSnapshot `json:"snapshot"`
	SHA256      string       `json:"sha256"`
	OperationID string       `json:"operation_id"`
	CreatedBy   string       `json:"created_by"`
	CreatedAt   time.Time    `json:"created_at"`
}
type OrderRecord struct {
	Snapshot          OrderSnapshot `json:"snapshot"`
	SHA256            string        `json:"sha256"`
	OperationID       string        `json:"operation_id"`
	Status            string        `json:"status"`
	CreatedBy         string        `json:"created_by"`
	CreatedAt         time.Time     `json:"created_at"`
	CustomerName      string        `json:"customer_name,omitempty"`
	FulfillmentID     string        `json:"fulfillment_id,omitempty"`
	FulfillmentStatus string        `json:"fulfillment_status,omitempty"`
}
