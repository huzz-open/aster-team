package mariadb

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"crypto/x509"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/adapters/licensing"
	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
)

type paidTestReferences struct {
	calls  atomic.Int32
	prefix string
}

func (r *paidTestReferences) Reference(customer string) (string, error) {
	r.calls.Add(1)
	return r.prefix + customer, nil
}

func testPaidFulfillmentLifecycle(t *testing.T, ctx context.Context, db *sql.DB, store *Store, actor, suffix string) {
	t.Helper()
	now := time.Now().UTC().Truncate(time.Millisecond)
	raw, err := os.ReadFile("../../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var definition commercial.Definition
	if err := json.Unmarshal(raw, &definition); err != nil {
		t.Fatal(err)
	}
	definition.Code = "paid_" + suffix
	planID, customer := "paid_plan_"+suffix, "paid_customer_"+suffix
	if _, err := store.FreezePlanVersion(ctx, planID, 0, definition, planID, actor, now); err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, `INSERT INTO customers(id,name,status,notes,created_at,updated_at) VALUES(?,?,'active','paid test',?,?)`, customer, "Paid test", now, now); err != nil {
		t.Fatal(err)
	}
	raw, err = os.ReadFile("../../../../../contracts/test-vectors/license-request.v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Request json.RawMessage `json:"request"`
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	request, err := licenseprotocol.ParseRequestV2(fixture.Request)
	if err != nil {
		t.Fatal(err)
	}
	request.ProductVersion = definition.MinimumVersion
	request.GeneratedAt = now.Add(-time.Second).Format("2006-01-02T15:04:05.000Z")
	// Every fixture creates a real immutable order and a real full payment.
	newOrder := func(label, customerID string, starts time.Time) (commercial.OrderRecord, commercial.PaymentRecord, commercial.ApprovePaidFulfillmentInput) {
		t.Helper()
		id := "pf_" + label + "_" + suffix
		created := time.Now().UTC().Truncate(time.Millisecond)
		order, err := store.CreateCommercialOrder(ctx, id, id, customerID, planID, 1, 1, starts, actor, created)
		if err != nil {
			t.Fatal(label, err)
		}
		in := commercial.ConfirmPaymentInput{OperationID: "pay_" + id, ExpectedOrderSHA256: order.SHA256, PaymentReference: "bank_" + id, ReceivedAt: created.Format("2006-01-02T15:04:05.000Z"), Notes: "isolated paid fixture"}
		payment, err := store.ConfirmCommercialPayment(ctx, "pay_"+id, id, in, actor)
		if err != nil {
			t.Fatal(label, err)
		}
		req := request
		req.RequestID = "request_" + suffix
		requestRaw, _ := json.Marshal(req)
		approval := commercial.ApprovePaidFulfillmentInput{OperationID: "approve_" + id, ExpectedOrderSHA256: order.SHA256, ExpectedPaymentSHA256: payment.SHA256, LicenseRequestJSON: string(requestRaw), Reason: "isolated paid approval"}
		return order, payment, approval
	}
	auditFailure := func(operation, action, resource string) func() {
		t.Helper()
		id := operationAuditID(operation, action)
		if _, err := db.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'test.injected_failure','commercial',?,'{}',?)`, id, actor, resource, time.Now().UTC()); err != nil {
			t.Fatal(err)
		}
		return func() {
			if _, err := db.ExecContext(ctx, "DELETE FROM audit_events WHERE id=? AND action='test.injected_failure'", id); err != nil {
				t.Fatal(err)
			}
		}
	}
	order, payment, in := newOrder("first", customer, now)
	identity := sha256.Sum256([]byte(actor + "\x00" + in.OperationID))
	orderID, id := order.Snapshot.OrderID, fmt.Sprintf("fulfillment_%x", identity[:24])
	refs := &paidTestReferences{prefix: "ref_"}
	if _, err := store.ApprovePaidFulfillment(ctx, id, orderID, in, "local", actor, nil); !errors.Is(err, commercial.ErrCustomerReferenceUnavailable) {
		t.Fatal("new approval without derivation", err)
	}
	if _, err := store.ApprovePaidFulfillment(ctx, id, orderID, in, "", actor, refs); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("new approval without environment", err)
	}
	clear := auditFailure(in.OperationID, "commercial.fulfillment_approved", id)
	if _, err := store.ApprovePaidFulfillment(ctx, id, orderID, in, "local", actor, refs); err == nil {
		t.Fatal("approval audit failure ignored")
	}
	if _, err := store.GetPaidFulfillment(ctx, id); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("partial approval persisted", err)
	}
	clear()
	var wg sync.WaitGroup
	results := make(chan error, 6)
	for range 6 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := store.ApprovePaidFulfillment(ctx, id, orderID, in, "local", actor, refs)
			results <- err
		}()
	}
	wg.Wait()
	close(results)
	for err := range results {
		if err != nil {
			t.Fatal("same operation concurrent recovery", err)
		}
	}
	original, err := store.GetPaidFulfillment(ctx, id)
	if err != nil {
		t.Fatal(err)
	}
	if err := original.ValidatePaymentSource(payment); err != nil {
		t.Fatal(err)
	}
	rotated := &paidTestReferences{prefix: "rotated_"}
	for _, source := range []ports.CustomerReferenceSource{rotated, nil} {
		got, err := store.ApprovePaidFulfillment(ctx, id, orderID, in, "production", actor, source)
		if err != nil || got.SHA256 != original.SHA256 || got.Snapshot.Environment != "local" {
			t.Fatal("original approval not restored", err)
		}
	}
	if rotated.calls.Load() != 0 {
		t.Fatal("committed recovery derived a new customer reference")
	}
	other := in
	other.OperationID += "_different"
	if _, err := store.ApprovePaidFulfillment(ctx, "other_"+suffix, orderID, other, "local", actor, refs); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("second initial approval", err)
	}
	for _, mutate := range []func(*commercial.ApprovePaidFulfillmentInput){func(v *commercial.ApprovePaidFulfillmentInput) { v.Reason += " changed" }, func(v *commercial.ApprovePaidFulfillmentInput) { v.LicenseRequestJSON += " " }, func(v *commercial.ApprovePaidFulfillmentInput) { v.ExpectedPaymentSHA256 = strings.Repeat("0", 64) }} {
		bad := in
		mutate(&bad)
		if _, err := store.ApprovePaidFulfillment(ctx, id, orderID, bad, "local", actor, refs); !errors.Is(err, commercial.ErrConflict) {
			t.Fatal("changed original input recovered", err)
		}
	}
	// PAID-R02: current legacy collation accepts this alias through ordering and payment.
	// Refuse it before derivation/commit; never rewrite the already frozen sale.
	aliasOrder, aliasPayment, aliasInput := newOrder("alias", strings.ToUpper(customer), now)
	beforeCalls := refs.calls.Load()
	for range 2 {
		if _, err := store.ApprovePaidFulfillment(ctx, "alias_"+suffix, aliasOrder.Snapshot.OrderID, aliasInput, "local", actor, refs); !errors.Is(err, commercial.ErrConflict) {
			t.Fatal("customer alias committed", err)
		}
	}
	if refs.calls.Load() != beforeCalls {
		t.Fatal("alias reached reference derivation")
	}
	if _, err := store.GetPaidFulfillmentForOrder(ctx, aliasOrder.Snapshot.OrderID); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("alias left unreadable approval", err)
	}
	unchanged, err := store.GetCommercialOrder(ctx, aliasOrder.Snapshot.OrderID)
	if err != nil || unchanged.SHA256 != aliasOrder.SHA256 || unchanged.Snapshot.CustomerID != strings.ToUpper(customer) {
		t.Fatal("alias sale silently rewritten", err)
	}
	unchangedPayment, err := store.GetCommercialPayment(ctx, aliasOrder.Snapshot.OrderID)
	if err != nil || unchangedPayment.SHA256 != aliasPayment.SHA256 {
		t.Fatal("alias receipt rewritten", err)
	}
	// The same request ID cannot identify another full machine request.
	second, _, secondInput := newOrder("request", customer, now)
	changedRequest := original.Snapshot.InstallationRequest
	changedRequest.InstallationID += "_other"
	raw, _ = json.Marshal(changedRequest)
	secondInput.LicenseRequestJSON = string(raw)
	if _, err := store.ApprovePaidFulfillment(ctx, "request_"+suffix, second.Snapshot.OrderID, secondInput, "local", actor, refs); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("request ID rebound to another machine", err)
	}
	secondInput.LicenseRequestJSON = in.LicenseRequestJSON
	if _, err := store.ApprovePaidFulfillment(ctx, "request_"+suffix, second.Snapshot.OrderID, secondInput, "local", actor, refs); err != nil {
		t.Fatal("same canonical request/customer rejected", err)
	}
	// Independent same-order operations have exactly one winner.
	race, _, raceInput := newOrder("race", customer, now)
	results = make(chan error, 4)
	for i := range 4 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			v := raceInput
			v.OperationID += fmt.Sprintf("_%d", i)
			_, err := store.ApprovePaidFulfillment(ctx, fmt.Sprintf("race%d_%s", i, suffix), race.Snapshot.OrderID, v, "local", actor, refs)
			results <- err
		}()
	}
	wg.Wait()
	close(results)
	success := 0
	for err := range results {
		if err == nil {
			success++
		} else if !errors.Is(err, commercial.ErrConflict) {
			t.Fatal("different operation race", err)
		}
	}
	if success != 1 {
		t.Fatal("multiple initial approvals", success)
	}

	// Two different customers concurrently first-use one request ID. The indexed
	// request range must serialize them even though neither order exists in it yet.
	rivalCustomer := "rival_" + suffix
	if _, err := db.ExecContext(ctx, `INSERT INTO customers(id,name,status,notes,created_at,updated_at) VALUES(?,?,'active','request race',?,?)`, rivalCustomer, "Rival test", now, now); err != nil {
		t.Fatal(err)
	}
	left, _, leftInput := newOrder("reql", customer, now)
	right, _, rightInput := newOrder("reqr", rivalCustomer, now)
	raceOrders := []commercial.OrderRecord{left, right}
	raceInputs := []commercial.ApprovePaidFulfillmentInput{leftInput, rightInput}
	raceIDs := []string{"reqleft_" + suffix, "reqright_" + suffix}
	results = make(chan error, 2)
	for i := range 2 {
		req := request
		req.RequestID = "same_request_" + suffix
		req.InstallationID += fmt.Sprintf("_%d", i)
		raw, _ := json.Marshal(req)
		raceInputs[i].LicenseRequestJSON = string(raw)
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := store.ApprovePaidFulfillment(ctx, raceIDs[i], raceOrders[i].Snapshot.OrderID, raceInputs[i], "local", actor, refs)
			results <- err
		}()
	}
	wg.Wait()
	close(results)
	success = 0
	for err := range results {
		if err == nil {
			success++
		} else if !errors.Is(err, commercial.ErrConflict) {
			t.Fatal("request identity race", err)
		}
	}
	if success != 1 {
		t.Fatal("request range admitted conflicting first uses", success)
	}
	for _, raceID := range raceIDs {
		_, err := store.GetPaidFulfillment(ctx, raceID)
		expected := 0
		if err == nil {
			expected = 1
		} else if !errors.Is(err, commercial.ErrNotFound) {
			t.Fatal(err)
		}
		var audits int
		if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE resource_id=? AND action='commercial.fulfillment_approved'", raceID).Scan(&audits); err != nil || audits != expected {
			t.Fatal("losing request left approval audit", err, audits, expected)
		}
	}
	// Fixed preparation and all subsequent writes retain their approval environment.
	if _, err := store.PreparePaidFulfillment(ctx, id, "paid-test", "production", actor); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("cross-environment prepare", err)
	}
	clear = auditFailure(id, "commercial.fulfillment_prepared", id)
	if _, err := store.PreparePaidFulfillment(ctx, id, "paid-test", "local", actor); err == nil {
		t.Fatal("prepare audit failure ignored")
	}
	prepared, err := store.GetPaidFulfillment(ctx, id)
	if err != nil || prepared.Status != "approved" {
		t.Fatal("partial preparation", err)
	}
	clear()
	prepared, err = store.PreparePaidFulfillment(ctx, id, "paid-test", "local", actor)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.PreparePaidFulfillment(ctx, id, "changed-key", "local", actor); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("prepared key changed", err)
	}
	private := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{29}, 32))
	document, err := licenseprotocol.SignV2(*prepared.Claims, private)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.CompletePaidFulfillment(ctx, id, document, "production", actor); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("cross-environment completion", err)
	}
	clear = auditFailure(id, "commercial.fulfillment_issued", id)
	if _, err := store.CompletePaidFulfillment(ctx, id, document, "local", actor); err == nil {
		t.Fatal("completion audit failure ignored")
	}
	current, err := store.GetCommercialOrder(ctx, orderID)
	if err != nil || current.Status != "fulfillment_pending" {
		t.Fatal("partial order completion", err)
	}
	retained, err := store.GetPaidFulfillment(ctx, id)
	if err != nil || retained.Status != "prepared" || retained.Document != nil {
		t.Fatal("partial signed artifact persisted", err)
	}
	clear()
	// Exercise application verification and actual storage together with only a paid-scoped key.
	privateDER, _ := x509.MarshalPKCS8PrivateKey(private)
	policy := licenseprotocol.IssuerPolicyV2{Sources: []licenseprotocol.SourceKindV2{licenseprotocol.CommercialOrderV2}, Bindings: []licenseprotocol.BindingKindV2{licenseprotocol.InstallationV2}, Expiries: []licenseprotocol.ExpiryKindV2{licenseprotocol.FixedExpiryV2}, EntitlementCeiling: definition.Entitlements}
	signer, err := licensing.NewV2("paid-test", base64.RawURLEncoding.EncodeToString(privateDER), policy)
	if err != nil {
		t.Fatal(err)
	}
	crossEnvironment := application.NewService(store, time.Hour, application.WithFulfillmentEnvironment("production"), application.WithV2LicenseSigners(map[string]ports.LicenseSignerV2{"paid-test": signer}))
	if _, err := crossEnvironment.IssuePaidFulfillment(ctx, id, "paid-test", actor); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("application signing crossed approval environment", err)
	}
	service := application.NewService(store, time.Hour, application.WithFulfillmentEnvironment("local"), application.WithV2LicenseSigners(map[string]ports.LicenseSignerV2{"paid-test": signer}), application.WithCustomerReferenceSource(refs))
	issued, err := service.IssuePaidFulfillment(ctx, id, "paid-test", actor)
	if err != nil || issued.Status != "issued" {
		t.Fatal("paid application issuance", err)
	}
	want, _ := json.Marshal(document)
	got, _ := json.Marshal(issued.Document)
	if !bytes.Equal(want, got) {
		t.Fatal("retry regenerated frozen claims or signature")
	}
	current, err = store.GetCommercialOrder(ctx, orderID)
	if err != nil || current.Status != "fulfilled" {
		t.Fatal("issued order not completed", err)
	}
	publicOnly, err := licensing.NewV2Verifier(signer.Profile())
	if err != nil {
		t.Fatal(err)
	}
	readOnly := application.NewService(store, time.Hour, application.WithFulfillmentEnvironment("production"), application.WithV2LicenseVerifiers(map[string]ports.LicenseVerifierV2{"paid-test": publicOnly}))
	for _, call := range []func() (commercial.PaidFulfillmentRecord, error){
		func() (commercial.PaidFulfillmentRecord, error) { return readOnly.GetPaidFulfillment(ctx, id, actor) },
		func() (commercial.PaidFulfillmentRecord, error) {
			return readOnly.IssuePaidFulfillment(ctx, id, "paid-test", actor)
		},
		func() (commercial.PaidFulfillmentRecord, error) {
			return readOnly.ApprovePaidFulfillment(ctx, orderID, in, actor)
		},
	} {
		r, err := call()
		if err != nil || r.DocumentSHA256 != issued.DocumentSHA256 {
			t.Fatal("public-only historical read/recovery", err)
		}
	}
	redeliveryID := "redelivery_" + suffix
	redeliveryInput := commercial.RecordPaidRedeliveryInput{OperationID: "redeliver_" + suffix, ExpectedDocumentSHA256: issued.DocumentSHA256, Reason: "customer requested the exact original file"}
	if _, err := store.RecordPaidRedelivery(ctx, redeliveryID, id, redeliveryInput, "production", actor); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("new redelivery crossed fulfillment environment", err)
	}
	clear = auditFailure(redeliveryInput.OperationID, "commercial.fulfillment_redelivery_recorded", redeliveryID)
	if _, err := store.RecordPaidRedelivery(ctx, redeliveryID, id, redeliveryInput, "local", actor); err == nil {
		t.Fatal("redelivery audit failure ignored")
	}
	if _, err := store.GetPaidRedelivery(ctx, redeliveryID); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("partial redelivery persisted", err)
	}
	clear()
	results = make(chan error, 6)
	for range 6 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := store.RecordPaidRedelivery(ctx, redeliveryID, id, redeliveryInput, "local", actor)
			results <- err
		}()
	}
	wg.Wait()
	close(results)
	for err := range results {
		if err != nil {
			t.Fatal("same redelivery operation concurrent recovery", err)
		}
	}
	redelivery, err := store.GetPaidRedelivery(ctx, redeliveryID)
	if err != nil || redelivery.Snapshot.DocumentSHA256 != issued.DocumentSHA256 || redelivery.ValidateFulfillment(issued) != nil {
		t.Fatal("original document identity was not preserved", err)
	}
	recoveredRedelivery, err := store.RecordPaidRedelivery(ctx, redeliveryID, id, redeliveryInput, "production", actor)
	if err != nil || recoveredRedelivery.SHA256 != redelivery.SHA256 || recoveredRedelivery.Snapshot.Environment != "local" {
		t.Fatal("historical redelivery recovery changed environment or record", err)
	}
	changedRedelivery := redeliveryInput
	changedRedelivery.Reason = "changed reason"
	if _, err := store.RecordPaidRedelivery(ctx, redeliveryID, id, changedRedelivery, "local", actor); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("same redelivery operation admitted changed input", err)
	}
	secondRedeliveryInput := redeliveryInput
	secondRedeliveryInput.OperationID = "redeliver_again_" + suffix
	localVerifier := application.NewService(store, time.Hour, application.WithFulfillmentEnvironment("local"), application.WithV2LicenseVerifiers(map[string]ports.LicenseVerifierV2{"paid-test": publicOnly}))
	secondRedelivery, err := localVerifier.RecordPaidRedelivery(ctx, id, secondRedeliveryInput, actor)
	if err != nil || secondRedelivery.Snapshot.DocumentSHA256 != issued.DocumentSHA256 {
		t.Fatal("second exact-file redelivery", err)
	}
	if _, err := readOnly.RecordPaidRedelivery(ctx, id, commercial.RecordPaidRedeliveryInput{OperationID: "cross_redeliver_" + suffix, ExpectedDocumentSHA256: issued.DocumentSHA256, Reason: "wrong environment"}, actor); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("application redelivery crossed fulfillment environment", err)
	}
	var redeliveries, redeliveryAudits int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_paid_redeliveries WHERE fulfillment_id=?", id).Scan(&redeliveries); err != nil || redeliveries != 2 {
		t.Fatal("unexpected redelivery records", err, redeliveries)
	}
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE action='commercial.fulfillment_redelivery_recorded' AND resource_id IN (?,?)", redeliveryID, secondRedelivery.Snapshot.ID).Scan(&redeliveryAudits); err != nil || redeliveryAudits != 2 {
		t.Fatal("unexpected redelivery audits", err, redeliveryAudits)
	}
	transferRequest := issued.Snapshot.InstallationRequest
	transferRequest.RequestID = "transfer_request_" + suffix
	transferRequest.InstallationID += "_moved"
	transferRequest.MachineFingerprintSHA256 = strings.Repeat("A", 43)
	transferRequest.GeneratedAt = time.Now().UTC().Add(-time.Second).Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")
	transferRaw, _ := json.Marshal(transferRequest)
	transferInput := commercial.ApprovePaidTransferInput{OperationID: "transfer_" + suffix, ExpectedCurrentDocumentSHA256: issued.DocumentSHA256, LicenseRequestJSON: string(transferRaw), Reason: "approved replacement machine"}
	transferIdentity := sha256.Sum256([]byte(actor + "\x00" + transferInput.OperationID))
	transferID := fmt.Sprintf("transfer_%x", transferIdentity[:24])
	clear = auditFailure(transferInput.OperationID, "commercial.fulfillment_transfer_approved", transferID)
	if _, err := service.ApprovePaidTransfer(ctx, id, transferInput, actor); err == nil {
		t.Fatal("transfer approval audit failure ignored")
	}
	if _, err := store.GetPaidTransfer(ctx, transferID); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("partial transfer approval persisted", err)
	}
	clear()
	approvedTransfer, err := service.ApprovePaidTransfer(ctx, id, transferInput, actor)
	if err != nil || approvedTransfer.Snapshot.TransferSequence != 1 || approvedTransfer.Snapshot.PreviousDocumentSHA256 != issued.DocumentSHA256 {
		t.Fatal("paid transfer approval", err)
	}
	if recovered, err := service.ApprovePaidTransfer(ctx, id, transferInput, actor); err != nil || recovered.SHA256 != approvedTransfer.SHA256 {
		t.Fatal("paid transfer approval recovery", err)
	}
	clear = auditFailure(transferID, "commercial.fulfillment_transfer_prepared", transferID)
	if _, err := service.IssuePaidTransfer(ctx, transferID, "paid-test", actor); err == nil {
		t.Fatal("transfer preparation audit failure ignored")
	}
	clear()
	issuedTransfer, err := service.IssuePaidTransfer(ctx, transferID, "paid-test", actor)
	if err != nil || issuedTransfer.Status != "issued" || issuedTransfer.Claims == nil || issuedTransfer.Claims.Binding.TransferSequence == nil || *issuedTransfer.Claims.Binding.TransferSequence != 1 {
		t.Fatal("paid transfer issuance", err)
	}
	if issuedTransfer.Claims.Validity != issued.Claims.Validity || issuedTransfer.Claims.LicenseID != issued.Claims.LicenseID {
		t.Fatal("transfer changed original license identity or contract dates")
	}
	if _, err := readOnly.GetPaidTransfer(ctx, transferID, actor); err != nil {
		t.Fatal("public-only transfer verification", err)
	}
	if _, err := readOnly.ApprovePaidTransfer(ctx, id, transferInput, actor); err != nil {
		t.Fatal("historical transfer recovery depended on current environment", err)
	}
	replayOrder, _, replayApproval := newOrder("transfer_replay", customer, now)
	replayApproval.LicenseRequestJSON = string(transferRaw)
	if _, err := store.ApprovePaidFulfillment(ctx, "transfer_replay_"+suffix, replayOrder.Snapshot.OrderID, replayApproval, "local", actor, refs); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("transfer request was replayed as an initial fulfillment", err)
	}
	initialAsTransfer := commercial.ApprovePaidTransferInput{OperationID: "initial_as_transfer_" + suffix, ExpectedCurrentDocumentSHA256: issuedTransfer.DocumentSHA256, LicenseRequestJSON: in.LicenseRequestJSON, Reason: "must not cross request usage"}
	if _, err := service.ApprovePaidTransfer(ctx, id, initialAsTransfer, actor); err == nil {
		t.Fatal("initial fulfillment request was replayed as a transfer")
	}
	sharedRequestID := "cross_usage_race_" + suffix
	sharedRequestSHA := strings.Repeat("1", 64)
	claimResults := make(chan error, 2)
	for _, usage := range []string{"initial", "transfer"} {
		wg.Add(1)
		go func() {
			defer wg.Done()
			err := retryCommercial(ctx, func() error {
				tx, err := db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
				if err != nil {
					return err
				}
				defer tx.Rollback()
				if err := claimCommercialInstallationRequest(ctx, tx, sharedRequestID, sharedRequestSHA, customer, usage, time.Now().UTC()); err != nil {
					return err
				}
				return tx.Commit()
			})
			claimResults <- err
		}()
	}
	wg.Wait()
	close(claimResults)
	claimSuccess, claimConflict := 0, 0
	for err := range claimResults {
		if err == nil {
			claimSuccess++
		} else if errors.Is(err, commercial.ErrConflict) {
			claimConflict++
		} else {
			t.Fatal("cross-usage request race", err)
		}
	}
	if claimSuccess != 1 || claimConflict != 1 {
		t.Fatal("cross-usage request race did not have exactly one winner", claimSuccess, claimConflict)
	}
	secondTransferRequest := transferRequest
	secondTransferRequest.RequestID = "transfer_request_second_" + suffix
	secondTransferRequest.InstallationID += "_again"
	secondTransferRequest.MachineFingerprintSHA256 = strings.Repeat("B", 43)
	secondTransferRequest.GeneratedAt = time.Now().UTC().Add(-time.Second).Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")
	transferRaw, _ = json.Marshal(secondTransferRequest)
	secondTransferInput := commercial.ApprovePaidTransferInput{OperationID: "transfer_second_" + suffix, ExpectedCurrentDocumentSHA256: issuedTransfer.DocumentSHA256, LicenseRequestJSON: string(transferRaw), Reason: "second approved replacement"}
	secondTransfer, err := service.ApprovePaidTransfer(ctx, id, secondTransferInput, actor)
	if err != nil {
		t.Fatal("second transfer approval", err)
	}
	secondTransfer, err = service.IssuePaidTransfer(ctx, secondTransfer.Snapshot.ID, "paid-test", actor)
	if err != nil || secondTransfer.Claims == nil || secondTransfer.Claims.Binding.TransferSequence == nil || *secondTransfer.Claims.Binding.TransferSequence != definition.TransferLimit {
		t.Fatal("second transfer did not consume frozen limit", err)
	}
	thirdTransferInput := secondTransferInput
	thirdTransferInput.OperationID = "transfer_third_" + suffix
	thirdTransferInput.ExpectedCurrentDocumentSHA256 = secondTransfer.DocumentSHA256
	thirdTransferRequest := secondTransferRequest
	thirdTransferRequest.RequestID = "transfer_request_third_" + suffix
	thirdTransferRequest.InstallationID += "_third"
	transferRaw, _ = json.Marshal(thirdTransferRequest)
	thirdTransferInput.LicenseRequestJSON = string(transferRaw)
	if _, err := service.ApprovePaidTransfer(ctx, id, thirdTransferInput, actor); err == nil {
		t.Fatal("transfer above frozen limit was approved")
	}
	currentExpiry, _ := time.Parse(time.RFC3339Nano, secondTransfer.Claims.Validity.Expiry.ExpiresAt)
	renewalOrder, _, renewalInput := newOrder("renewal", customer, currentExpiry)
	renewalRequest := secondTransfer.Snapshot.InstallationRequest
	renewalRequest.RequestID = "renewal_request_" + suffix
	renewalRequest.GeneratedAt = time.Now().UTC().Add(-time.Second).Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")
	renewalRaw, _ := json.Marshal(renewalRequest)
	renewalInput.LicenseRequestJSON = string(renewalRaw)
	renewalInput.Lifecycle = &commercial.PaidLifecycleRequest{Kind: commercial.PaidLifecycleRenewal, SourceID: id, ExpectedDocumentSHA256: secondTransfer.DocumentSHA256}
	preview, err := store.GetPaidLifecycleSource(ctx, commercial.PaidLifecycleRenewal, id, "local")
	if err != nil || preview.SourceRecordKind != "paid_transfer" || preview.SourceRecordID != secondTransfer.Snapshot.ID || preview.DocumentSHA256 != secondTransfer.DocumentSHA256 {
		t.Fatal("renewal preview did not resolve the current transfer", err)
	}
	if _, err := store.GetPaidLifecycleSource(ctx, commercial.PaidLifecycleRenewal, id, "production"); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("cross-environment lifecycle preview admitted", err)
	}
	renewalID := "renewal_" + suffix
	renewal, err := service.ApprovePaidFulfillment(ctx, renewalOrder.Snapshot.OrderID, renewalInput, actor)
	if err != nil || renewal.Snapshot.Lifecycle == nil || renewal.Snapshot.Lifecycle.SourceRecordID != secondTransfer.Snapshot.ID {
		t.Fatal("renewal approval", err)
	}
	renewalID = renewal.Snapshot.ID
	if _, err := store.GetPaidLifecycleSource(ctx, commercial.PaidLifecycleRenewal, id, "local"); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("already frozen predecessor remained available", err)
	}
	// Frozen lifecycle recovery verifies the signed predecessor and immutable
	// source row, but no longer depends on today's environment or HMAC secret.
	rotatedOperations := application.NewService(store, time.Hour, application.WithFulfillmentEnvironment("production"), application.WithV2LicenseSigners(map[string]ports.LicenseSignerV2{"paid-test": signer}))
	for _, recover := range []func() (commercial.PaidFulfillmentRecord, error){
		func() (commercial.PaidFulfillmentRecord, error) {
			return rotatedOperations.GetPaidFulfillment(ctx, renewalID, actor)
		},
		func() (commercial.PaidFulfillmentRecord, error) {
			return rotatedOperations.ApprovePaidFulfillment(ctx, renewalOrder.Snapshot.OrderID, renewalInput, actor)
		},
	} {
		recovered, recoverErr := recover()
		if recoverErr != nil || recovered.SHA256 != renewal.SHA256 {
			t.Fatal("frozen lifecycle recovery depended on current configuration", recoverErr)
		}
	}
	blockedTransferRequest := secondTransferRequest
	blockedTransferRequest.RequestID = "blocked_after_renewal_" + suffix
	blockedTransferRequest.InstallationID += "_blocked"
	blockedRaw, _ := json.Marshal(blockedTransferRequest)
	blockedTransfer := commercial.ApprovePaidTransferInput{OperationID: "blocked_after_renewal_" + suffix, ExpectedCurrentDocumentSHA256: secondTransfer.DocumentSHA256, LicenseRequestJSON: string(blockedRaw), Reason: "must not move after successor approval"}
	if _, err := service.ApprovePaidTransfer(ctx, id, blockedTransfer, actor); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("predecessor moved after renewal approval", err)
	}
	renewal, err = rotatedOperations.IssuePaidFulfillment(ctx, renewalID, "paid-test", actor)
	if err != nil || renewal.Status != "issued" || renewal.Claims == nil || renewal.Claims.LicenseID == secondTransfer.Claims.LicenseID || renewal.Claims.Binding.InstallationID != secondTransfer.Claims.Binding.InstallationID || renewal.Claims.Validity.NotBefore != secondTransfer.Claims.Validity.Expiry.ExpiresAt {
		t.Fatal("renewal issuance did not preserve continuity", err)
	}
	secondRenewalOrder, _, secondRenewalInput := newOrder("renewal_duplicate", customer, currentExpiry)
	secondRenewalRequest := renewalRequest
	secondRenewalRequest.RequestID = "renewal_duplicate_request_" + suffix
	secondRenewalRaw, _ := json.Marshal(secondRenewalRequest)
	secondRenewalInput.LicenseRequestJSON = string(secondRenewalRaw)
	secondRenewalInput.Lifecycle = &commercial.PaidLifecycleRequest{Kind: commercial.PaidLifecycleRenewal, SourceID: id, ExpectedDocumentSHA256: secondTransfer.DocumentSHA256}
	if _, err := store.ApprovePaidFulfillment(ctx, "renewal_duplicate_"+suffix, secondRenewalOrder.Snapshot.OrderID, secondRenewalInput, "local", actor, refs); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("predecessor produced a second successor", err)
	}
	if _, err := store.GetPaidLifecycleSource(ctx, "trial_conversion", "old_internal_trial", "local"); !errors.Is(err, commercial.ErrInvalidPaidLifecycle) {
		t.Fatal("legacy trial source accepted", err)
	}
	var transfers, transferApprovalAudits, transferIssueAudits int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_paid_transfers WHERE fulfillment_id=?", id).Scan(&transfers); err != nil || transfers != 2 {
		t.Fatal("unexpected transfer records", err, transfers)
	}
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE action='commercial.fulfillment_transfer_approved' AND resource_id IN (?,?)", transferID, secondTransfer.Snapshot.ID).Scan(&transferApprovalAudits); err != nil || transferApprovalAudits != 2 {
		t.Fatal("unexpected transfer approval audits", err, transferApprovalAudits)
	}
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE action='commercial.fulfillment_transfer_issued' AND resource_id IN (?,?)", transferID, secondTransfer.Snapshot.ID).Scan(&transferIssueAudits); err != nil || transferIssueAudits != 2 {
		t.Fatal("unexpected transfer issue audits", err, transferIssueAudits)
	}
	// A self-consistent document digest does not replace trusted signature verification.
	forged := *issued.Document
	forged.Signature = base64.RawURLEncoding.EncodeToString(make([]byte, ed25519.SignatureSize))
	forgedRaw, _ := json.Marshal(forged)
	if _, err := db.ExecContext(ctx, "UPDATE commercial_paid_fulfillments SET document_json=?,document_sha256=? WHERE id=?", forgedRaw, commercial.ContentDigest(forgedRaw), id); err != nil {
		t.Fatal(err)
	}
	if _, err := readOnly.GetPaidFulfillment(ctx, id, actor); err == nil {
		t.Fatal("forged signature served with matching self-digest")
	}
	if _, err := readOnly.IssuePaidFulfillment(ctx, id, "paid-test", actor); err == nil {
		t.Fatal("forged signature recovered")
	}
	forgedRedelivery := commercial.RecordPaidRedeliveryInput{OperationID: "forged_redelivery_" + suffix, ExpectedDocumentSHA256: commercial.ContentDigest(forgedRaw), Reason: "must fail before audit"}
	if _, err := localVerifier.RecordPaidRedelivery(ctx, id, forgedRedelivery, actor); err == nil {
		t.Fatal("forged signature produced a redelivery record")
	}
	var forgedRecords, forgedAudits int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_paid_redeliveries WHERE operation_id=?", forgedRedelivery.OperationID).Scan(&forgedRecords); err != nil || forgedRecords != 0 {
		t.Fatal("forged signature left a redelivery row", err, forgedRecords)
	}
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE action='commercial.fulfillment_redelivery_recorded' AND resource_id LIKE 'redelivery_%' AND payload_json LIKE ?", "%"+forgedRedelivery.OperationID+"%").Scan(&forgedAudits); err != nil || forgedAudits != 0 {
		t.Fatal("forged signature left a redelivery audit", err, forgedAudits)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_paid_fulfillments SET document_json=?,document_sha256=? WHERE id=?", got, issued.DocumentSHA256, id); err != nil {
		t.Fatal(err)
	}
	noKeys := application.NewService(store, time.Hour)
	if _, err := noKeys.GetPaidFulfillment(ctx, id, actor); !errors.Is(err, application.ErrV2VerifierUnavailable) {
		t.Fatal("missing public key bypassed verification", err)
	}
	var approvals, issuedAudits int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_paid_fulfillments WHERE order_id=?", orderID).Scan(&approvals); err != nil || approvals != 1 {
		t.Fatal("duplicate approval", err, approvals)
	}
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE resource_id=? AND action='commercial.fulfillment_issued'", id).Scan(&issuedAudits); err != nil || issuedAudits != 1 {
		t.Fatal("duplicate issue audit", err, issuedAudits)
	}
	// A locally valid rewritten snapshot must still equal its real immutable payment.
	sourceID := "request_" + suffix
	sourceRecord, err := store.GetPaidFulfillment(ctx, sourceID)
	if err != nil {
		t.Fatal(err)
	}
	originalSource, _ := sourceRecord.Snapshot.Bytes()
	forgedSource := sourceRecord
	forgedSource.Snapshot.Payment.Snapshot.Request.Notes += " forged"
	paymentRaw, _ := forgedSource.Snapshot.Payment.Snapshot.Bytes()
	forgedSource.Snapshot.Payment.SHA256 = commercial.ContentDigest(paymentRaw)
	forgedSource.Snapshot.Request.ExpectedPaymentSHA256 = forgedSource.Snapshot.Payment.SHA256
	changedRaw, err := forgedSource.Snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	forgedSource.SHA256 = commercial.ContentDigest(changedRaw)
	if err := forgedSource.Validate(); err != nil {
		t.Fatal("counterexample must be self-consistent", err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_paid_fulfillments SET snapshot_json=?,content_sha256=?,payment_sha256=? WHERE id=?", changedRaw, forgedSource.SHA256, forgedSource.Snapshot.Payment.SHA256, sourceID); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetPaidFulfillment(ctx, sourceID); !errors.Is(err, commercial.ErrPaidFulfillmentIntegrity) {
		t.Fatal("self-consistent forged source read", err)
	}
	if _, err := store.ApprovePaidFulfillment(ctx, sourceID, second.Snapshot.OrderID, forgedSource.Snapshot.Request, "local", actor, nil); !errors.Is(err, commercial.ErrPaidFulfillmentIntegrity) {
		t.Fatal("self-consistent forged source recovered", err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_paid_fulfillments SET snapshot_json=?,content_sha256=?,payment_sha256=? WHERE id=?", originalSource, sourceRecord.SHA256, sourceRecord.Snapshot.Payment.SHA256, sourceID); err != nil {
		t.Fatal(err)
	}
	// Existing prepared claims finish after expiry; first approval waiting on a lock cannot.
	end := time.Now().UTC().Add(4 * time.Second).Truncate(time.Millisecond)
	expiring, _, expiringInput := newOrder("expiry", customer, end.AddDate(-1, 0, 0))
	expiredPreparedID := "expired_" + suffix
	if _, err := store.ApprovePaidFulfillment(ctx, expiredPreparedID, expiring.Snapshot.OrderID, expiringInput, "local", actor, refs); err != nil {
		t.Fatal(err)
	}
	frozen, err := store.PreparePaidFulfillment(ctx, expiredPreparedID, "paid-test", "local", actor)
	if err != nil {
		t.Fatal(err)
	}
	frozenDoc, err := licenseprotocol.SignV2(*frozen.Claims, private)
	if err != nil {
		t.Fatal(err)
	}
	waiting, _, waitingInput := newOrder("lock", customer, end.AddDate(-1, 0, 0))
	lock, err := db.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer lock.Rollback()
	var lockedCustomer string
	if err := lock.QueryRowContext(ctx, "SELECT id FROM customers WHERE id=? FOR UPDATE", customer).Scan(&lockedCustomer); err != nil {
		t.Fatal(err)
	}
	waitingResult := make(chan error, 1)
	go func() {
		_, err := store.ApprovePaidFulfillment(ctx, "wait_"+suffix, waiting.Snapshot.OrderID, waitingInput, "local", actor, refs)
		waitingResult <- err
	}()
	select {
	case <-time.After(time.Until(end.Add(50 * time.Millisecond))):
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	if err := lock.Rollback(); err != nil {
		t.Fatal(err)
	}
	if err := <-waitingResult; err == nil {
		t.Fatal("first approval committed after waiting beyond expiry")
	}
	if _, err := store.GetPaidFulfillmentForOrder(ctx, waiting.Snapshot.OrderID); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("expired first approval persisted", err)
	}
	// The predecessor expiry, rather than only the new order expiry, bounds all
	// upgrade and transfer lock waits. Both attempts below observe the immutable
	// source before blocking on its row and must roll back without side effects.
	boundaryEnd := time.Now().UTC().Add(5 * time.Second).Truncate(time.Millisecond)
	boundarySourceOrder, _, boundarySourceInput := newOrder("source_deadline", customer, boundaryEnd.AddDate(-1, 0, 0))
	boundarySourceID := "source_deadline_" + suffix
	boundarySource, err := store.ApprovePaidFulfillment(ctx, boundarySourceID, boundarySourceOrder.Snapshot.OrderID, boundarySourceInput, "local", actor, refs)
	if err != nil {
		t.Fatal("approve expiring boundary source", err)
	}
	boundarySource, err = store.PreparePaidFulfillment(ctx, boundarySourceID, "paid-test", "local", actor)
	if err != nil {
		t.Fatal("prepare expiring boundary source", err)
	}
	boundaryDocument, err := licenseprotocol.SignV2(*boundarySource.Claims, private)
	if err != nil {
		t.Fatal(err)
	}
	boundarySource, err = store.CompletePaidFulfillment(ctx, boundarySourceID, boundaryDocument, "local", actor)
	if err != nil {
		t.Fatal("issue expiring boundary source", err)
	}
	upgradeOrder, _, upgradeInput := newOrder("source_deadline_upgrade", customer, time.Now().UTC().Truncate(time.Millisecond))
	upgradeRequest := boundarySource.Snapshot.InstallationRequest
	upgradeRequest.RequestID = "source_deadline_upgrade_request_" + suffix
	upgradeRequest.GeneratedAt = time.Now().UTC().Add(-time.Second).Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")
	upgradeRaw, _ := json.Marshal(upgradeRequest)
	upgradeInput.LicenseRequestJSON = string(upgradeRaw)
	upgradeInput.Lifecycle = &commercial.PaidLifecycleRequest{Kind: commercial.PaidLifecycleUpgrade, SourceID: boundarySourceID, ExpectedDocumentSHA256: boundarySource.DocumentSHA256}
	deadlineTransferRequest := boundarySource.Snapshot.InstallationRequest
	deadlineTransferRequest.RequestID = "source_deadline_transfer_request_" + suffix
	deadlineTransferRequest.InstallationID += "_deadline"
	deadlineTransferRequest.MachineFingerprintSHA256 = strings.Repeat("E", 43)
	deadlineTransferRequest.GeneratedAt = time.Now().UTC().Add(-time.Second).Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")
	deadlineTransferRaw, _ := json.Marshal(deadlineTransferRequest)
	deadlineTransferInput := commercial.ApprovePaidTransferInput{OperationID: "source_deadline_transfer_" + suffix, ExpectedCurrentDocumentSHA256: boundarySource.DocumentSHA256, LicenseRequestJSON: string(deadlineTransferRaw), Reason: "must not cross predecessor expiry"}
	deadlineTransferID := "source_deadline_transfer_record_" + suffix
	boundaryLock, err := db.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer boundaryLock.Rollback()
	var lockedSource string
	if err := boundaryLock.QueryRowContext(ctx, "SELECT id FROM commercial_paid_fulfillments WHERE id=? FOR UPDATE", boundarySourceID).Scan(&lockedSource); err != nil {
		t.Fatal(err)
	}
	boundaryResults := make(chan error, 2)
	go func() {
		_, approveErr := store.ApprovePaidFulfillment(ctx, "source_deadline_upgrade_"+suffix, upgradeOrder.Snapshot.OrderID, upgradeInput, "local", actor, refs)
		boundaryResults <- approveErr
	}()
	go func() {
		_, approveErr := store.ApprovePaidTransfer(ctx, deadlineTransferID, boundarySourceID, deadlineTransferInput, "local", actor)
		boundaryResults <- approveErr
	}()
	select {
	case <-time.After(time.Until(boundaryEnd.Add(50 * time.Millisecond))):
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	if err := boundaryLock.Rollback(); err != nil {
		t.Fatal(err)
	}
	for range 2 {
		if err := <-boundaryResults; err == nil {
			t.Fatal("source-bound approval committed after predecessor expiry")
		}
	}
	var boundaryRecords, boundaryAudits, boundaryRequests int
	if err := db.QueryRowContext(ctx, "SELECT (SELECT COUNT(*) FROM commercial_paid_fulfillments WHERE operation_id=?) + (SELECT COUNT(*) FROM commercial_paid_transfers WHERE operation_id=?)", upgradeInput.OperationID, deadlineTransferInput.OperationID).Scan(&boundaryRecords); err != nil {
		t.Fatal(err)
	}
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE (resource_id=? AND action='commercial.fulfillment_approved') OR (resource_id=? AND action='commercial.fulfillment_transfer_approved')", "source_deadline_upgrade_"+suffix, deadlineTransferID).Scan(&boundaryAudits); err != nil {
		t.Fatal(err)
	}
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_installation_requests WHERE request_id IN (?,?)", upgradeRequest.RequestID, deadlineTransferRequest.RequestID).Scan(&boundaryRequests); err != nil || boundaryRecords != 0 || boundaryAudits != 0 || boundaryRequests != 0 {
		t.Fatal("expired predecessor left approval side effects", err, boundaryRecords, boundaryAudits, boundaryRequests)
	}
	if _, err := store.PreparePaidFulfillment(ctx, expiredPreparedID, "paid-test", "production", actor); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("expired prepared environment changed", err)
	}
	afterExpiry, err := store.PreparePaidFulfillment(ctx, expiredPreparedID, "paid-test", "local", actor)
	if err != nil || afterExpiry.Claims.IssuedAt != frozen.Claims.IssuedAt {
		t.Fatal("frozen preparation extended on retry", err)
	}
	if _, err := store.CompletePaidFulfillment(ctx, expiredPreparedID, frozenDoc, "production", actor); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("expired completion environment changed", err)
	}
	expiredReceipt, err := store.CompletePaidFulfillment(ctx, expiredPreparedID, frozenDoc, "local", actor)
	if err != nil || expiredReceipt.Claims.Validity.Expiry.ExpiresAt != end.Format("2006-01-02T15:04:05.000Z") {
		t.Fatal("same-environment expiry recovery", err)
	}

}

func testPaidQuotationFulfillment(t *testing.T, ctx context.Context, db *sql.DB, store *Store, order commercial.OrderRecord, actor, suffix string) {
	t.Helper()
	if order.Snapshot.Schema != commercial.QuotationOrderSchema || order.Snapshot.Source == nil {
		t.Fatal("requires actual quoted v2 order")
	}
	payment, err := store.GetCommercialPayment(ctx, order.Snapshot.OrderID)
	if err != nil {
		t.Fatal(err)
	}
	source, err := store.GetPublication(ctx, order.Snapshot.Source.PublicationID)
	if err != nil {
		t.Fatal(err)
	}
	deadline, err := time.Parse(time.RFC3339Nano, source.Snapshot.Request.AcceptUntil)
	if err != nil || time.Now().Before(deadline) {
		t.Fatal("quotation acceptance must really have expired", err)
	}
	data, err := os.ReadFile("../../../../../contracts/test-vectors/license-request.v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Request json.RawMessage `json:"request"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	request, err := licenseprotocol.ParseRequestV2(fixture.Request)
	if err != nil {
		t.Fatal(err)
	}
	request.RequestID = "quoted_request_" + suffix
	request.ProductVersion = order.Snapshot.Plan.Definition.MinimumVersion
	request.GeneratedAt = time.Now().UTC().Add(-time.Second).Format("2006-01-02T15:04:05.000Z")
	raw, _ := json.Marshal(request)
	input := commercial.ApprovePaidFulfillmentInput{OperationID: "quoted_approve_" + suffix, ExpectedOrderSHA256: order.SHA256, ExpectedPaymentSHA256: payment.SHA256, LicenseRequestJSON: string(raw), Reason: "expired accepted quotation remains original sale"}
	refs := &paidTestReferences{prefix: "quote_ref_"}
	id := "quoted_fulfillment_" + suffix
	if _, err := store.ApprovePaidFulfillment(ctx, id, order.Snapshot.OrderID, input, "production", actor, refs); !errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		t.Fatal("local quotation promoted to production", err)
	}
	if _, err := store.ApprovePaidFulfillment(ctx, id, order.Snapshot.OrderID, input, "local", actor, refs); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("inactive customer approved", err)
	}
	if refs.calls.Load() != 0 {
		t.Fatal("rejected quotation reached derivation")
	}
	if _, err := db.ExecContext(ctx, "UPDATE customers SET status='active' WHERE id=?", order.Snapshot.CustomerID); err != nil {
		t.Fatal(err)
	}
	approved, err := store.ApprovePaidFulfillment(ctx, id, order.Snapshot.OrderID, input, "local", actor, refs)
	if err != nil {
		t.Fatal("original quotation lost after acceptance expiry", err)
	}
	if approved.Snapshot.Payment.Snapshot.Order.Source.PublicationSHA256 != source.SHA256 || approved.Snapshot.Payment.Snapshot.OrderSHA256 != order.SHA256 {
		t.Fatal("quoted fulfillment lost original source")
	}
	prepared, err := store.PreparePaidFulfillment(ctx, id, "quoted-paid-test", "local", actor)
	if err != nil {
		t.Fatal(err)
	}
	document, err := licenseprotocol.SignV2(*prepared.Claims, ed25519.NewKeyFromSeed(bytes.Repeat([]byte{30}, 32)))
	if err != nil {
		t.Fatal(err)
	}
	issued, err := store.CompletePaidFulfillment(ctx, id, document, "local", actor)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE customers SET status='inactive' WHERE id=?", order.Snapshot.CustomerID); err != nil {
		t.Fatal(err)
	}
	recovered, err := store.ApprovePaidFulfillment(ctx, id, order.Snapshot.OrderID, input, "production", actor, nil)
	if err != nil || recovered.DocumentSHA256 != issued.DocumentSHA256 || recovered.Snapshot.Environment != "local" {
		t.Fatal("historical quoted fulfillment recovery changed", err)
	}
	if got, err := store.GetPaidFulfillment(ctx, id); err != nil || got.DocumentSHA256 != issued.DocumentSHA256 {
		t.Fatal("historical quoted fulfillment unreadable", err)
	}
}
