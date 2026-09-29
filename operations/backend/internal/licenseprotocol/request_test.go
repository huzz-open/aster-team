package licenseprotocol

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestRequestRejectsAliasesDuplicatesAndTrailingGarbage(t *testing.T) {
	raw := string(validRequestJSON(t))
	for name, bad := range map[string]string{
		"trailing invalid": raw + "x",
		"trailing JSON":    raw + "{}",
		"top alias":        strings.Replace(raw, `"request_id":`, `"Request_ID":`, 1),
		"top duplicate":    strings.Replace(raw, `"request_id":`, `"request_id":"different-request","request_id":`, 1),
		"nested alias":     strings.Replace(raw, `"kind":`, `"Kind":`, 1),
		"nested duplicate": strings.Replace(raw, `"kind":`, `"kind":"machine_id","kind":`, 1),
		"nested missing":   strings.Replace(raw, `"kind":"machine_id",`, "", 1),
		"null factors":     strings.Replace(raw, `"machine_factors":[`, `"machine_factors":null,"forged":[`, 1),
	} {
		t.Run(name, func(t *testing.T) {
			if bad == raw {
				t.Fatal("test did not mutate fixture")
			}
			if _, err := ParseRequest([]byte(bad)); err == nil {
				t.Fatal("parser accepted malformed request")
			}
			var v Request
			if err := json.Unmarshal([]byte(bad), &v); err == nil {
				t.Fatal("direct unmarshal accepted malformed request")
			}
		})
	}
	parsed, err := ParseRequest([]byte(raw))
	if err != nil {
		t.Fatal(err)
	}
	var direct Request
	if err := json.Unmarshal([]byte(raw), &direct); err != nil || direct.RequestID != parsed.RequestID {
		t.Fatal("valid direct decode", err)
	}
}
