package licenseprotocol

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"
)

func TestRequestV2SharedCompatibilityAndStrictParsing(t *testing.T) {
	data, err := os.ReadFile("../../../../contracts/test-vectors/license-request.v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Request   json.RawMessage `json:"request"`
		Canonical string          `json:"canonical_request"`
		Invalid   []struct {
			Name string `json:"name"`
			Raw  string `json:"raw"`
		} `json:"invalid_raw"`
		Versions []struct {
			Actual   string `json:"actual"`
			Minimum  string `json:"minimum"`
			Expected bool   `json:"expected"`
		} `json:"versions"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	request, err := ParseRequestV2(fixture.Request)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ParseRequest(fixture.Request); err == nil {
		t.Fatal("v2 silently accepted by v1")
	}
	if _, err := ParseRequestV2(validRequestJSON(t)); err == nil {
		t.Fatal("v1 promoted to v2")
	}
	encoded, _ := json.Marshal(request)
	var value any
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	decoder.UseNumber()
	if err := decoder.Decode(&value); err != nil {
		t.Fatal(err)
	}
	canonical, err := Canonicalize(value)
	if err != nil || string(canonical) != fixture.Canonical {
		t.Fatal("shared bytes differ", err)
	}
	for _, item := range fixture.Invalid {
		t.Run(item.Name, func(t *testing.T) {
			if _, err := ParseRequestV2([]byte(item.Raw)); err == nil {
				t.Fatal("invalid request accepted")
			}
			var direct RequestV2
			if json.Unmarshal([]byte(item.Raw), &direct) == nil {
				t.Fatal("direct decode accepted invalid request")
			}
		})
	}
	for _, item := range fixture.Versions {
		if actual := VersionAtLeastV2(item.Actual, item.Minimum); actual != item.Expected {
			t.Errorf("%s >= %s: got %v", item.Actual, item.Minimum, actual)
		}
	}
	if !request.Supports("1.2.3+other", 1, 1) || request.Supports("1.2.4", 1, 1) || request.Supports("1.2.3", 2, 1) || request.Supports("1.2.3", 1, 2) {
		t.Fatal("compatibility requirements not enforced")
	}
}
