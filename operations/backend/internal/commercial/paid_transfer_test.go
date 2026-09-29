package commercial

import (
	"bytes"
	"crypto/ed25519"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/licenseprotocol"
)

func transferInput(t *testing.T, fulfillment PaidFulfillmentRecord, operation string, generated time.Time) ApprovePaidTransferInput {
	t.Helper()
	request := fulfillment.Snapshot.InstallationRequest
	request.RequestID = "transfer_request_" + operation
	request.InstallationID += "_" + operation
	request.MachineFingerprintSHA256 = strings.Repeat("A", 43)
	request.GeneratedAt = generated.UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")
	raw, err := json.Marshal(request)
	if err != nil {
		t.Fatal(err)
	}
	return ApprovePaidTransferInput{OperationID: operation, ExpectedCurrentDocumentSHA256: fulfillment.DocumentSHA256, LicenseRequestJSON: string(raw), Reason: "approved machine transfer"}
}

func issueTransfer(t *testing.T, record PaidTransferRecord, keyID string, now time.Time) PaidTransferRecord {
	t.Helper()
	claims, err := record.Snapshot.Claims(keyID, now)
	if err != nil {
		t.Fatal(err)
	}
	document, err := licenseprotocol.SignV2(claims, ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize)))
	if err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(document)
	if err != nil {
		t.Fatal(err)
	}
	record.Status, record.Claims, record.Document, record.DocumentSHA256 = "issued", &claims, &document, ContentDigest(raw)
	if err := record.Validate(); err != nil {
		t.Fatal(err)
	}
	return record
}

func TestPaidTransferPreservesSaleAndAdvancesOnlyBinding(t *testing.T) {
	fulfillment := issuedPaidFulfillmentFixture(t)
	previousClaims := *fulfillment.Claims
	previousIssued, _ := parseTime(previousClaims.IssuedAt)
	input := transferInput(t, fulfillment, "transfer_1", previousIssued.Add(time.Second))
	record, err := NewPaidTransfer("paid_transfer_1", input, fulfillment, previousClaims, fulfillment.DocumentSHA256, "operator_1", previousIssued.Add(2*time.Second))
	if err != nil {
		t.Fatal(err)
	}
	issued := issueTransfer(t, record, "paid_transfer_key", previousIssued.Add(3*time.Second))
	claims := issued.Claims
	if claims.LicenseID != fulfillment.Claims.LicenseID || claims.Source.OrderID != fulfillment.Claims.Source.OrderID || claims.Source.CustomerRef != fulfillment.Claims.Source.CustomerRef || claims.Source.RequestID != record.Snapshot.InstallationRequest.RequestID {
		t.Fatal("transfer changed sale identity or lost the new request")
	}
	if claims.Validity != fulfillment.Claims.Validity || claims.Edition != fulfillment.Claims.Edition || claims.PlanID != fulfillment.Claims.PlanID || claims.PlanVersion != fulfillment.Claims.PlanVersion || claims.MinimumVersion != fulfillment.Claims.MinimumVersion {
		t.Fatal("transfer changed the original commercial terms")
	}
	wantEntitlements, _ := canonical(fulfillment.Claims.Entitlements)
	gotEntitlements, _ := canonical(claims.Entitlements)
	if !bytes.Equal(wantEntitlements, gotEntitlements) || claims.Binding.TransferSequence == nil || *claims.Binding.TransferSequence != 1 {
		t.Fatal("transfer changed entitlements or sequence")
	}
	if claims.Binding.InstallationID != record.Snapshot.InstallationRequest.InstallationID || claims.Binding.MachineFingerprintSHA256 != record.Snapshot.InstallationRequest.MachineFingerprintSHA256 {
		t.Fatal("transfer did not bind the approved installation")
	}
	raw, _ := json.Marshal(issued)
	var decoded PaidTransferRecord
	if err := json.Unmarshal(raw, &decoded); err != nil || decoded.Validate() != nil {
		t.Fatal("issued transfer round trip", err)
	}
}

func TestPaidTransferEnforcesCurrentDocumentSequenceAndDeadline(t *testing.T) {
	fulfillment := issuedPaidFulfillmentFixture(t)
	start, _ := parseTime(fulfillment.Claims.IssuedAt)
	firstInput := transferInput(t, fulfillment, "transfer_1", start.Add(time.Second))
	first, err := NewPaidTransfer("paid_transfer_1", firstInput, fulfillment, *fulfillment.Claims, fulfillment.DocumentSHA256, "operator_1", start.Add(2*time.Second))
	if err != nil {
		t.Fatal(err)
	}
	first = issueTransfer(t, first, "paid_transfer_key", start.Add(3*time.Second))

	secondInput := transferInput(t, fulfillment, "transfer_2", start.Add(4*time.Second))
	secondInput.ExpectedCurrentDocumentSHA256 = first.DocumentSHA256
	second, err := NewPaidTransfer("paid_transfer_2", secondInput, fulfillment, *first.Claims, first.DocumentSHA256, "operator_1", start.Add(5*time.Second))
	if err != nil {
		t.Fatal(err)
	}
	second = issueTransfer(t, second, "paid_transfer_key", start.Add(6*time.Second))
	if second.Claims.Binding.TransferSequence == nil || *second.Claims.Binding.TransferSequence != 2 {
		t.Fatal("second transfer did not advance the sequence exactly once")
	}
	thirdInput := transferInput(t, fulfillment, "transfer_3", start.Add(7*time.Second))
	thirdInput.ExpectedCurrentDocumentSHA256 = second.DocumentSHA256
	exhausted := *second.Claims
	limit := fulfillment.Snapshot.Payment.Snapshot.Order.Plan.Definition.TransferLimit
	exhausted.Binding.TransferSequence = &limit
	if _, err := NewPaidTransfer("paid_transfer_3", thirdInput, fulfillment, exhausted, second.DocumentSHA256, "operator_1", start.Add(8*time.Second)); err == nil {
		t.Fatal("transfer above the frozen plan limit was accepted")
	}

	sameMachine := firstInput
	request := fulfillment.Snapshot.InstallationRequest
	request.RequestID = "same_machine_request"
	request.GeneratedAt = start.Add(time.Second).Format("2006-01-02T15:04:05.000Z")
	raw, _ := json.Marshal(request)
	sameMachine.LicenseRequestJSON = string(raw)
	if _, err := NewPaidTransfer("same_machine", sameMachine, fulfillment, *fulfillment.Claims, fulfillment.DocumentSHA256, "operator_1", start.Add(2*time.Second)); err == nil {
		t.Fatal("unchanged installation consumed a transfer")
	}

	expires, _ := parseTime(fulfillment.Claims.Validity.Expiry.ExpiresAt)
	late := transferInput(t, fulfillment, "late_transfer", expires.Add(-time.Second))
	if _, err := NewPaidTransfer("late_transfer", late, fulfillment, *fulfillment.Claims, fulfillment.DocumentSHA256, "operator_1", expires); err == nil {
		t.Fatal("transfer at the contract deadline was accepted")
	}

	var duplicate ApprovePaidTransferInput
	bad := `{"operation_id":"a","operation_id":"b","expected_current_document_sha256":"` + strings.Repeat("0", 64) + `","license_request_json":{},"reason":"x"}`
	if json.Unmarshal([]byte(bad), &duplicate) == nil {
		t.Fatal("ambiguous transfer input was accepted")
	}
}

func TestPaidTransferBindsCompactAuthorityToFulfillment(t *testing.T) {
	fulfillment := issuedPaidFulfillmentFixture(t)
	start, _ := parseTime(fulfillment.Claims.IssuedAt)
	input := transferInput(t, fulfillment, "transfer_1", start.Add(time.Second))
	record, err := NewPaidTransfer("paid_transfer_1", input, fulfillment, *fulfillment.Claims, fulfillment.DocumentSHA256, "operator_1", start.Add(2*time.Second))
	if err != nil || record.ValidateFulfillment(fulfillment) != nil {
		t.Fatal("valid compact authority was not bound to fulfillment", err)
	}
	tampered := record
	tampered.Snapshot.TransferLimit++
	raw, _ := tampered.Snapshot.Bytes()
	tampered.SHA256 = ContentDigest(raw)
	if tampered.ValidateFulfillment(fulfillment) == nil {
		t.Fatal("self-consistent changed transfer authority was accepted")
	}
}
