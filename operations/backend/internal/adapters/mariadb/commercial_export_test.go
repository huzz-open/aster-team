package mariadb

import (
	"bytes"
	"encoding/json"
	"testing"
)

func TestCommercialExportProjectsNestedLicenseDocumentsWithoutChangingNumbers(t *testing.T) {
	input := []byte(`{"schema":"aster.paid-fulfillment.v2","amount":9007199254740993,"lifecycle":{"document_sha256":"digest","document":{"schema":"aster.license.v2","signature":"private-delivery"}},"history":[{"document":{"claims":{}},"status":"issued"}]}`)
	projected, err := projectCommercialExportSnapshot(input)
	if err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range [][]byte{[]byte(`"document":`), []byte("private-delivery"), []byte("aster.license.v2")} {
		if bytes.Contains(projected, forbidden) {
			t.Fatalf("export contains signed document: %s", projected)
		}
	}
	if !bytes.Contains(projected, []byte(`9007199254740993`)) || !bytes.Contains(projected, []byte(`"document_sha256":"digest"`)) {
		t.Fatalf("projection lost business values: %s", projected)
	}
	var value map[string]json.RawMessage
	if err := json.Unmarshal(projected, &value); err != nil {
		t.Fatal(err)
	}
	if string(value["schema"]) != `"aster.paid-fulfillment.v2"` {
		t.Fatal("schema changed")
	}
	if !bytes.Contains(input, []byte("private-delivery")) {
		t.Fatal("source snapshot was mutated")
	}
	for _, invalid := range []string{`null`, `[]`, `{`} {
		if _, err := projectCommercialExportSnapshot([]byte(invalid)); err == nil {
			t.Fatalf("accepted invalid snapshot %s", invalid)
		}
	}
}
