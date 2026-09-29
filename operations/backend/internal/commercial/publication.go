package commercial

import (
	"errors"
	"fmt"
	"net"
	"net/url"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/strictjson"
)

const PublicationSchema = "aster.catalog-publication.v1"
const MaximumPublicationBytes = 2 << 20

var ErrInvalidPublication = errors.New("invalid catalog publication")
var ErrPublicationNotAccepted = errors.New("public quotation is not accepted")
var ErrPublicationPreparationRejected = fmt.Errorf("%w: original publication was not stored and its head or deadline is no longer valid", ErrConflict)

func ValidatePublicationOrigin(environment, origin string) error {
	u, err := url.Parse(origin)
	if err != nil || len(origin) > 2048 || u.Host == "" || u.User != nil || u.Path != "" || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || u.Opaque != "" || u.String() != origin {
		return ErrInvalidPublication
	}
	ip := net.ParseIP(u.Hostname())
	host := strings.TrimSuffix(strings.ToLower(u.Hostname()), ".")
	local := host == "localhost" || strings.HasSuffix(host, ".localhost") || (ip != nil && ip.IsLoopback())
	if environment == "local" && local && (u.Scheme == "http" || u.Scheme == "https") {
		return nil
	}
	if environment == "production" && !local && u.Scheme == "https" && (ip == nil || (ip.IsGlobalUnicast() && !ip.IsPrivate())) {
		return nil
	}
	return ErrInvalidPublication
}

// A new event, including a rollback, has a new operation ID. The expected head
// prevents a stale reconciliation from replacing a newer accepted publication.
type PreparePublicationInput struct {
	OperationID      string `json:"operation_id"`
	CatalogRevision  string `json:"catalog_revision"`
	BuildSHA256      string `json:"build_sha256"`
	ExpectedActiveID string `json:"expected_active_id"`
	AcceptUntil      string `json:"accept_until"`
	Reason           string `json:"reason"`
}

func (v *PreparePublicationInput) UnmarshalJSON(data []byte) error {
	type wire PreparePublicationInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "catalog_revision", "build_sha256", "expected_active_id", "accept_until", "reason"); err != nil {
		return err
	}
	*v = PreparePublicationInput(result)
	return v.Validate()
}
func (v PreparePublicationInput) Validate() error {
	if !identifier.MatchString(v.OperationID) || !catalogRevision.MatchString(v.CatalogRevision) || !catalogDigest.MatchString(v.BuildSHA256) || (v.ExpectedActiveID != "" && (!identifier.MatchString(v.ExpectedActiveID) || len(v.ExpectedActiveID) > 64)) || !validText(v.Reason, 1000, false) {
		return ErrInvalidPublication
	}
	if _, err := parseTime(v.AcceptUntil); err != nil {
		return ErrInvalidPublication
	}
	return nil
}
func (v PreparePublicationInput) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	return canonical(v)
}

type PublicationSnapshot struct {
	Schema    string                  `json:"schema"`
	ID        string                  `json:"id"`
	Request   PreparePublicationInput `json:"request"`
	Catalog   CatalogApprovalSnapshot `json:"catalog"`
	CreatedBy string                  `json:"created_by"`
	CreatedAt string                  `json:"created_at"`
}

func (v *PublicationSnapshot) UnmarshalJSON(data []byte) error {
	type wire PublicationSnapshot
	var result wire
	if err := strictjson.ObjectWithin(data, &result, MaximumPublicationBytes, "schema", "id", "request", "catalog", "created_by", "created_at"); err != nil {
		return err
	}
	*v = PublicationSnapshot(result)
	_, err := v.Bytes()
	return err
}
func (v PublicationSnapshot) Bytes() ([]byte, error) {
	if v.Schema != PublicationSchema || !identifier.MatchString(v.ID) || len(v.ID) > 64 || !identifier.MatchString(v.CreatedBy) || len(v.CreatedBy) > 64 || v.Request.CatalogRevision != v.Catalog.ID || v.ID == v.Request.ExpectedActiveID {
		return nil, ErrInvalidPublication
	}
	if err := v.Request.Validate(); err != nil {
		return nil, err
	}
	if _, err := v.Catalog.Bytes(); err != nil {
		return nil, err
	}
	created, err := parseTime(v.CreatedAt)
	approved, approvalErr := parseTime(v.Catalog.ApprovedAt)
	until, untilErr := parseTime(v.Request.AcceptUntil)
	if err != nil || approvalErr != nil || untilErr != nil || created.Before(approved) || !created.Before(until) {
		return nil, ErrInvalidPublication
	}
	raw, err := canonical(v)
	if err != nil || len(raw) > MaximumPublicationBytes {
		return nil, ErrInvalidPublication
	}
	return raw, nil
}

// Evidence is obtained by an environment-bound server adapter, never accepted
// as part of a browser request. A local verifier cannot produce production proof.
type PublicationEvidence struct {
	Environment     string `json:"environment"`
	Origin          string `json:"origin"`
	BuildSHA256     string `json:"build_sha256"`
	CatalogRevision string `json:"catalog_revision"`
	CatalogSHA256   string `json:"catalog_sha256"`
	ObservedAt      string `json:"observed_at"`
}

func (v *PublicationEvidence) UnmarshalJSON(data []byte) error {
	type wire PublicationEvidence
	var result wire
	if err := strictjson.Object(data, &result, "environment", "origin", "build_sha256", "catalog_revision", "catalog_sha256", "observed_at"); err != nil {
		return err
	}
	*v = PublicationEvidence(result)
	return nil
}

type PublicationRecord struct {
	Snapshot   PublicationSnapshot  `json:"snapshot"`
	SHA256     string               `json:"sha256"`
	Status     string               `json:"status"`
	Evidence   *PublicationEvidence `json:"evidence,omitempty"`
	AcceptedBy string               `json:"accepted_by,omitempty"`
	AcceptedAt string               `json:"accepted_at,omitempty"`
}

func (v PublicationRecord) Validate() error {
	raw, err := v.Snapshot.Bytes()
	if err != nil || ContentDigest(raw) != v.SHA256 {
		return ErrInvalidPublication
	}
	if v.Status == "prepared" {
		if v.Evidence != nil || v.AcceptedBy != "" || v.AcceptedAt != "" {
			return ErrInvalidPublication
		}
		return nil
	}
	if v.Status != "accepted" || v.Evidence == nil || !identifier.MatchString(v.AcceptedBy) || len(v.AcceptedBy) > 64 {
		return ErrInvalidPublication
	}
	e := v.Evidence
	if e.Environment != v.Snapshot.Catalog.Request.Environment || e.BuildSHA256 != v.Snapshot.Request.BuildSHA256 || e.CatalogRevision != v.Snapshot.Catalog.ID || e.CatalogSHA256 != v.Snapshot.Catalog.PublicSHA256 || ValidatePublicationOrigin(e.Environment, e.Origin) != nil {
		return ErrInvalidPublication
	}
	observed, observedErr := parseTime(e.ObservedAt)
	accepted, acceptedErr := parseTime(v.AcceptedAt)
	created, _ := parseTime(v.Snapshot.CreatedAt)
	until, _ := parseTime(v.Snapshot.Request.AcceptUntil)
	if observedErr != nil || acceptedErr != nil || observed.Before(created) || accepted.Before(observed) || accepted.Sub(observed) > time.Minute || !accepted.Before(until) {
		return ErrInvalidPublication
	}
	return nil
}

// The caller supplies the trusted sales channel, not a visitor-selected
// environment. Historical accepted events stay usable only through their
// explicitly approved deadline; a new active head never rewrites their terms.
func (v PublicationRecord) ResolveQuotation(environment, revision, planID string, version, years uint32, now time.Time) (FrozenPlan, error) {
	if err := v.Validate(); err != nil {
		return FrozenPlan{}, err
	}
	until, _ := parseTime(v.Snapshot.Request.AcceptUntil)
	accepted, _ := parseTime(v.AcceptedAt)
	if v.Status != "accepted" || environment != v.Snapshot.Catalog.Request.Environment || revision != v.Snapshot.Catalog.ID || now.Before(accepted) || !now.Before(until) {
		return FrozenPlan{}, ErrPublicationNotAccepted
	}
	for _, plan := range v.Snapshot.Catalog.Plans {
		if plan.PlanID != planID || plan.Version != version || plan.Definition.Offer.Kind != "annual" {
			continue
		}
		for _, term := range plan.Definition.Offer.Terms {
			if term.Years == years {
				return FreezePlan(plan.PlanID, plan.Version, plan.Definition)
			}
		}
	}
	return FrozenPlan{}, ErrPublicationNotAccepted
}
