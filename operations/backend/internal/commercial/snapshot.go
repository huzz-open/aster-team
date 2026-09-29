// Package commercial fixes complete plan and order terms independently of mutable catalog heads.
package commercial

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"math/big"
	"regexp"
	"slices"
	"strings"
	"time"
	_ "time/tzdata" // Keep supported calendar rules available in minimal Operations bundles.
	"unicode"
	"unicode/utf8"

	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/productcatalog"
	"aster.local/team/operations/backend/internal/strictjson"
)

const PlanSchema = "aster.plan-snapshot.v1"
const OrderSchema = "aster.order-snapshot.v1"
const QuotationOrderSchema = "aster.order-snapshot.v2"
const MaximumAmountMinor int64 = 9_000_000_000_000

var identifier = regexp.MustCompile(`^[A-Za-z0-9._:@+/-]{1,128}$`)
var currency = regexp.MustCompile(`^[A-Z]{3}$`)

type Term struct {
	Years               uint32 `json:"years"`
	DiscountBasisPoints uint32 `json:"discount_basis_points"`
}

func (value *Term) UnmarshalJSON(data []byte) error {
	type wire Term
	var result wire
	if err := strictjson.Object(data, &result, "years", "discount_basis_points"); err != nil {
		return err
	}
	*value = Term(result)
	return nil
}

type Offer struct {
	Kind              string                    `json:"kind"`
	Currency          string                    `json:"currency,omitempty"`
	AnnualAmountMinor int64                     `json:"annual_amount_minor,omitempty"`
	TaxMode           string                    `json:"tax_mode,omitempty"`
	Terms             []Term                    `json:"terms,omitempty"`
	TermRule          string                    `json:"term_rule,omitempty"`
	TermTimezone      string                    `json:"term_timezone,omitempty"`
	Expiry            *licenseprotocol.ExpiryV2 `json:"expiry,omitempty"`
}

func (value *Offer) UnmarshalJSON(data []byte) error {
	var tag struct {
		Kind string `json:"kind"`
	}
	if err := json.Unmarshal(data, &tag); err != nil {
		return err
	}
	fields := []string{"kind"}
	switch tag.Kind {
	case "free":
		fields = append(fields, "expiry")
	case "annual":
		fields = append(fields, "currency", "annual_amount_minor", "tax_mode", "terms", "term_rule", "term_timezone")
	case "contact":
	default:
		return errors.New("unknown offering kind")
	}
	type wire Offer
	var result wire
	if err := strictjson.Object(data, &result, fields...); err != nil {
		return err
	}
	*value = Offer(result)
	return value.Validate()
}
func (value Offer) Validate() error {
	switch value.Kind {
	case "annual":
		if !currency.MatchString(value.Currency) || value.AnnualAmountMinor < 1 || value.AnnualAmountMinor > MaximumAmountMinor || value.Expiry != nil {
			return errors.New("invalid annual price")
		}
		if value.TaxMode != "inclusive" && value.TaxMode != "none" {
			return errors.New("v2 prices require an explicit final tax-inclusive or no-tax amount")
		}
		if value.TermRule != "calendar_years_clamp_day" || (value.TermTimezone != "UTC" && value.TermTimezone != "Asia/Shanghai") {
			return errors.New("unsupported term calendar")
		}
		if len(value.Terms) < 1 || len(value.Terms) > 5 {
			return errors.New("annual offering requires explicit supported terms")
		}
		seen := map[uint32]bool{}
		for _, term := range value.Terms {
			if term.Years < 1 || term.Years > 5 || term.DiscountBasisPoints < 1 || term.DiscountBasisPoints > 10000 || seen[term.Years] {
				return errors.New("invalid or duplicate term")
			}
			seen[term.Years] = true
			if _, err := CalculateAmount(value.AnnualAmountMinor, term); err != nil {
				return err
			}
		}
	case "free", "contact":
		if value.Currency != "" || value.AnnualAmountMinor != 0 || value.TaxMode != "" || len(value.Terms) != 0 || value.TermRule != "" || value.TermTimezone != "" {
			return errors.New("nonannual offering cannot carry prices or terms")
		}
		if value.Kind == "contact" {
			if value.Expiry != nil {
				return errors.New("contact offering cannot carry a free expiry")
			}
			return nil
		}
		if value.Expiry == nil {
			return errors.New("free offering requires an explicit expiry policy")
		}
		switch value.Expiry.Mode {
		case licenseprotocol.NoExpiryV2:
			if value.Expiry.ExpiresAt != "" {
				return errors.New("nonexpiring offering contains expiry")
			}
		case licenseprotocol.FixedExpiryV2:
			if _, err := parseTime(value.Expiry.ExpiresAt); err != nil {
				return err
			}
		default:
			return errors.New("unknown free expiry")
		}
	default:
		return errors.New("unknown offering kind")
	}
	return nil
}

type Definition struct {
	Product             string                      `json:"product"`
	Code                string                      `json:"code"`
	Name                string                      `json:"name"`
	Description         string                      `json:"description"`
	Edition             string                      `json:"edition"`
	Entitlements        productcatalog.Entitlements `json:"entitlements"`
	QuotaPolicyVersion  uint32                      `json:"quota_policy_version"`
	MinimumVersion      string                      `json:"minimum_version"`
	TransferLimit       uint32                      `json:"transfer_limit"`
	SupportTermsVersion string                      `json:"support_terms_version"`
	Offer               Offer                       `json:"offer"`
}

func (value *Definition) UnmarshalJSON(data []byte) error {
	type wire Definition
	var result wire
	if err := strictjson.Object(data, &result, "product", "code", "name", "description", "edition", "entitlements", "quota_policy_version", "minimum_version", "transfer_limit", "support_terms_version", "offer"); err != nil {
		return err
	}
	*value = Definition(result)
	return value.Validate()
}
func (value Definition) Validate() error {
	if value.Product != "aster-team" || !identifier.MatchString(value.Code) || !identifier.MatchString(value.Edition) || !identifier.MatchString(value.SupportTermsVersion) {
		return errors.New("invalid plan identity or support terms version")
	}
	if !validText(value.Name, 160, false) || !validText(value.Description, 4000, true) {
		return errors.New("invalid plan display text")
	}
	if value.QuotaPolicyVersion != licenseprotocol.QuotaPolicyVersionV2 || !licenseprotocol.ValidMinimumVersionV2(value.MinimumVersion) || value.TransferLimit > 10000 {
		return errors.New("invalid license policy metadata")
	}
	if err := value.Entitlements.Validate(); err != nil {
		return err
	}
	if value.Offer.Kind == "free" && len(value.Entitlements.FeatureSets) > 0 {
		return fmt.Errorf("free plans must declare explicit features")
	}
	return value.Offer.Validate()
}
func validText(value string, maximum int, empty bool) bool {
	if !utf8.ValidString(value) || len([]rune(value)) > maximum || strings.TrimSpace(value) != value || (!empty && value == "") {
		return false
	}
	return !strings.ContainsFunc(value, func(r rune) bool { return unicode.IsControl(r) && r != '\n' })
}

type PlanSnapshot struct {
	Schema     string     `json:"schema"`
	PlanID     string     `json:"plan_id"`
	Version    uint32     `json:"version"`
	Definition Definition `json:"definition"`
}

func (value *PlanSnapshot) UnmarshalJSON(data []byte) error {
	type wire PlanSnapshot
	var result wire
	if err := strictjson.Object(data, &result, "schema", "plan_id", "version", "definition"); err != nil {
		return err
	}
	*value = PlanSnapshot(result)
	return value.Validate()
}
func (value PlanSnapshot) Validate() error {
	if value.Schema != PlanSchema || !identifier.MatchString(value.PlanID) || len(value.PlanID) > 64 || value.Version == 0 {
		return errors.New("invalid plan snapshot identity")
	}
	return value.Definition.Validate()
}

// FrozenPlan owns its encoded snapshot. Returned projections are independent copies.
// A matching digest is content identity, not permission to publish or sell a plan.
type FrozenPlan struct {
	raw    []byte
	digest string
}

func FreezePlan(planID string, version uint32, definition Definition) (FrozenPlan, error) {
	snapshot := PlanSnapshot{Schema: PlanSchema, PlanID: planID, Version: version, Definition: definition}
	if err := snapshot.Validate(); err != nil {
		return FrozenPlan{}, err
	}
	// Stable order means UI selection order does not create a different version.
	snapshot.Definition.Entitlements = cloneEntitlements(snapshot.Definition.Entitlements)
	slices.Sort(snapshot.Definition.Entitlements.Features)
	slices.Sort(snapshot.Definition.Entitlements.FeatureSets)
	slices.SortFunc(snapshot.Definition.Entitlements.Quotas, func(a, b productcatalog.QuotaGrant) int { return strings.Compare(string(a.ID), string(b.ID)) })
	snapshot.Definition.Offer.Terms = slices.Clone(snapshot.Definition.Offer.Terms)
	slices.SortFunc(snapshot.Definition.Offer.Terms, func(a, b Term) int {
		if a.Years < b.Years {
			return -1
		}
		if a.Years > b.Years {
			return 1
		}
		return 0
	})
	raw, err := canonical(snapshot)
	if err != nil {
		return FrozenPlan{}, err
	}
	return FrozenPlan{raw: raw, digest: digest(raw)}, nil
}
func ParseFrozenPlan(data []byte, expectedDigest string) (FrozenPlan, error) {
	if len(data) > 1<<20 {
		return FrozenPlan{}, errors.New("plan snapshot too large")
	}
	var snapshot PlanSnapshot
	if err := json.Unmarshal(data, &snapshot); err != nil {
		return FrozenPlan{}, err
	}
	result, err := FreezePlan(snapshot.PlanID, snapshot.Version, snapshot.Definition)
	if err != nil {
		return FrozenPlan{}, err
	}
	if result.digest != expectedDigest {
		return FrozenPlan{}, errors.New("plan snapshot digest mismatch")
	}
	return result, nil
}
func (value FrozenPlan) Bytes() []byte  { return bytes.Clone(value.raw) }
func (value FrozenPlan) Digest() string { return value.digest }
func (value FrozenPlan) Snapshot() (PlanSnapshot, error) {
	var result PlanSnapshot
	if len(value.raw) == 0 {
		return result, errors.New("uninitialized plan snapshot")
	}
	err := json.Unmarshal(value.raw, &result)
	return result, err
}

func CalculateAmount(annualAmount int64, term Term) (int64, error) {
	if annualAmount < 1 || annualAmount > MaximumAmountMinor || term.Years < 1 || term.Years > 5 || term.DiscountBasisPoints < 1 || term.DiscountBasisPoints > 10000 {
		return 0, errors.New("invalid price calculation input")
	}
	result := big.NewInt(annualAmount)
	result.Mul(result, big.NewInt(int64(term.Years)))
	result.Mul(result, big.NewInt(int64(term.DiscountBasisPoints)))
	result.Add(result, big.NewInt(5000))
	result.Quo(result, big.NewInt(10000))
	if !result.IsInt64() || result.Int64() < 1 || result.Int64() > MaximumAmountMinor {
		return 0, errors.New("calculated price exceeds supported range")
	}
	return result.Int64(), nil
}
func EndOfTerm(starts time.Time, years uint32, rule, timezone string) (time.Time, error) {
	if starts.IsZero() || starts.Year() < 1 || years < 1 || years > 5 || starts.Year()+int(years) > 9999 || rule != "calendar_years_clamp_day" || (timezone != "UTC" && timezone != "Asia/Shanghai") {
		return time.Time{}, errors.New("unsupported term input")
	}
	location, err := time.LoadLocation(timezone)
	if err != nil {
		return time.Time{}, err
	}
	local := starts.In(location)
	year := local.Year() + int(years)
	lastDay := time.Date(year, local.Month()+1, 0, 0, 0, 0, 0, location).Day()
	day := min(local.Day(), lastDay)
	return time.Date(year, local.Month(), day, local.Hour(), local.Minute(), local.Second(), local.Nanosecond(), location).UTC(), nil
}

type OrderSnapshot struct {
	Schema              string                `json:"schema"`
	OrderID             string                `json:"order_id"`
	CustomerID          string                `json:"customer_id"`
	Plan                PlanSnapshot          `json:"plan"`
	PlanSHA256          string                `json:"plan_sha256"`
	Years               uint32                `json:"years"`
	DiscountBasisPoints uint32                `json:"discount_basis_points"`
	AmountMinor         int64                 `json:"amount_minor"`
	Currency            string                `json:"currency"`
	TaxMode             string                `json:"tax_mode"`
	StartsAt            string                `json:"starts_at"`
	EndsAt              string                `json:"ends_at"`
	Source              *QuotationOrderSource `json:"source,omitempty"`
}

func (value *OrderSnapshot) UnmarshalJSON(data []byte) error {
	type wire OrderSnapshot
	var result wire
	var tag struct {
		Schema string `json:"schema"`
	}
	if err := json.Unmarshal(data, &tag); err != nil {
		return err
	}
	fields := []string{"schema", "order_id", "customer_id", "plan", "plan_sha256", "years", "discount_basis_points", "amount_minor", "currency", "tax_mode", "starts_at", "ends_at"}
	if tag.Schema == QuotationOrderSchema {
		fields = append(fields, "source")
	}
	if err := strictjson.Object(data, &result, fields...); err != nil {
		return err
	}
	*value = OrderSnapshot(result)
	return value.Validate()
}
func CreateOrderSnapshot(orderID, customerID string, plan FrozenPlan, years uint32, starts time.Time) (OrderSnapshot, error) {
	if !identifier.MatchString(orderID) || len(orderID) > 64 || !identifier.MatchString(customerID) || len(customerID) > 64 || starts.Nanosecond()%1_000_000 != 0 {
		return OrderSnapshot{}, errors.New("invalid order identity")
	}
	snapshot, err := plan.Snapshot()
	if err != nil {
		return OrderSnapshot{}, err
	}
	if snapshot.Definition.Offer.Kind != "annual" {
		return OrderSnapshot{}, errors.New("only annual offerings can create a priced order; contact requires an approved custom quote")
	}
	offer := snapshot.Definition.Offer
	var selected *Term
	for _, term := range offer.Terms {
		if term.Years == years {
			copy := term
			selected = &copy
			break
		}
	}
	if selected == nil {
		return OrderSnapshot{}, errors.New("term is not offered by this plan version")
	}
	amount, err := CalculateAmount(offer.AnnualAmountMinor, *selected)
	if err != nil {
		return OrderSnapshot{}, err
	}
	end, err := EndOfTerm(starts, years, offer.TermRule, offer.TermTimezone)
	if err != nil {
		return OrderSnapshot{}, err
	}
	result := OrderSnapshot{Schema: OrderSchema, OrderID: orderID, CustomerID: customerID, Plan: snapshot, PlanSHA256: plan.Digest(), Years: years, DiscountBasisPoints: selected.DiscountBasisPoints, AmountMinor: amount, Currency: offer.Currency, TaxMode: offer.TaxMode, StartsAt: starts.UTC().Format("2006-01-02T15:04:05.000Z"), EndsAt: end.UTC().Format("2006-01-02T15:04:05.000Z")}
	return result, result.Validate()
}
func (value OrderSnapshot) Validate() error {
	if (value.Schema != OrderSchema && value.Schema != QuotationOrderSchema) || !identifier.MatchString(value.OrderID) || len(value.OrderID) > 64 || !identifier.MatchString(value.CustomerID) || len(value.CustomerID) > 64 {
		return errors.New("invalid order snapshot identity")
	}
	if value.Schema == QuotationOrderSchema {
		if value.Source == nil || value.Source.Validate() != nil {
			return ErrInvalidQuotationOrder
		}
	} else if value.Source != nil {
		return ErrInvalidQuotationOrder
	}
	frozen, err := FreezePlan(value.Plan.PlanID, value.Plan.Version, value.Plan.Definition)
	if err != nil {
		return err
	}
	if value.Plan.Schema != PlanSchema || value.PlanSHA256 != frozen.Digest() {
		return errors.New("order refers to a different plan snapshot")
	}
	offer := value.Plan.Definition.Offer
	if offer.Kind != "annual" || value.Currency != offer.Currency || value.TaxMode != offer.TaxMode {
		return errors.New("order price metadata differs from plan")
	}
	found := false
	for _, term := range offer.Terms {
		if term.Years == value.Years && term.DiscountBasisPoints == value.DiscountBasisPoints {
			found = true
		}
	}
	if !found {
		return errors.New("order term differs from plan")
	}
	amount, err := CalculateAmount(offer.AnnualAmountMinor, Term{Years: value.Years, DiscountBasisPoints: value.DiscountBasisPoints})
	if err != nil || amount != value.AmountMinor {
		return errors.New("order amount differs from fixed calculation")
	}
	starts, err := parseTime(value.StartsAt)
	if err != nil {
		return err
	}
	ends, err := parseTime(value.EndsAt)
	if err != nil {
		return err
	}
	expected, err := EndOfTerm(starts, value.Years, offer.TermRule, offer.TermTimezone)
	if err != nil || !ends.Equal(expected) {
		return errors.New("order dates differ from fixed term")
	}
	return nil
}
func (value OrderSnapshot) Bytes() ([]byte, error) {
	if err := value.Validate(); err != nil {
		return nil, err
	}
	return canonical(value)
}
func ParseOrderSnapshot(data []byte, expectedDigest string) (OrderSnapshot, error) {
	if len(data) > 1<<20 {
		return OrderSnapshot{}, errors.New("order snapshot too large")
	}
	var value OrderSnapshot
	if err := json.Unmarshal(data, &value); err != nil {
		return value, err
	}
	raw, err := value.Bytes()
	if err != nil {
		return value, err
	}
	if digest(raw) != expectedDigest {
		return OrderSnapshot{}, errors.New("order snapshot digest mismatch")
	}
	return value, nil
}
func ContentDigest(data []byte) string { return digest(data) }
func digest(data []byte) string        { sum := sha256.Sum256(data); return hex.EncodeToString(sum[:]) }
func canonical(value any) ([]byte, error) {
	data, err := json.Marshal(value)
	if err != nil {
		return nil, err
	}
	var raw any
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err := decoder.Decode(&raw); err != nil {
		return nil, err
	}
	return licenseprotocol.Canonicalize(raw)
}
func parseTime(value string) (time.Time, error) {
	parsed, err := time.Parse(time.RFC3339Nano, value)
	if err != nil || parsed.UTC().Format("2006-01-02T15:04:05.000Z") != value {
		return time.Time{}, fmt.Errorf("invalid exact UTC time")
	}
	return parsed, nil
}
func cloneEntitlements(value productcatalog.Entitlements) productcatalog.Entitlements {
	value.Features = slices.Clone(value.Features)
	value.FeatureSets = slices.Clone(value.FeatureSets)
	value.Quotas = slices.Clone(value.Quotas)
	for i := range value.Quotas {
		if p := value.Quotas[i].Limit.Value; p != nil {
			n := *p
			value.Quotas[i].Limit.Value = &n
		}
	}
	return value
}
