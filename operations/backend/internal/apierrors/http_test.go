package apierrors

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strconv"
	"testing"
)

func TestWriteNormalizesUnregisteredCodeToPublishedInternalError(t *testing.T) {
	response := httptest.NewRecorder()
	number := Write(response, http.StatusBadRequest, "INVALID_INPUT", "invalid input")
	if number != 69_001 {
		t.Fatalf("number = %d, want published internal error 69001", number)
	}
	if response.Header().Get(NumberHeader) != strconv.FormatInt(number, 10) {
		t.Fatalf("header = %q, want %d", response.Header().Get(NumberHeader), number)
	}
	var body struct {
		Error struct {
			Code    string `json:"code"`
			Message string `json:"message"`
			Number  int64  `json:"number"`
		} `json:"error"`
	}
	if err := json.Unmarshal(response.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if body.Error.Code != "INTERNAL_ERROR" || body.Error.Message != "服务发生内部错误" || body.Error.Number != number {
		t.Fatalf("body = %+v", body)
	}
}

func TestNumberForUnknownCodeUsesPublishedInternalError(t *testing.T) {
	if got := NumberForCode("INVALID_INPUT"); got != 69_001 {
		t.Fatalf("unknown code = %d, want 69001", got)
	}
}

func TestNumbersAreSeparatedByEndpointAndBusinessModule(t *testing.T) {
	if got := NumberForCode("INVALID_CREDENTIALS"); got != 61_002 {
		t.Fatalf("INVALID_CREDENTIALS = %d, want 61002", got)
	}
	if got := NumberForCode("CUSTOMER_CREATE_FAILED"); got != 62_002 {
		t.Fatalf("CUSTOMER_CREATE_FAILED = %d, want 62002", got)
	}
	if got := NumberForCode("API_UNAVAILABLE"); got != 66_001 {
		t.Fatalf("API_UNAVAILABLE = %d, want 66001", got)
	}
	if got := NumberForCode("VALIDATION_FAILED"); got != 69_006 {
		t.Fatalf("VALIDATION_FAILED = %d, want 69006", got)
	}
}

func TestKnownCodesRemainStableAcrossRepeatedLookups(t *testing.T) {
	for range 1000 {
		if got := NumberForCode("NOT_FOUND"); got != 69_007 {
			t.Fatalf("NOT_FOUND = %d, want stable Operations code 69007", got)
		}
		if got := NumberForCode("API_UNAVAILABLE"); got != 66_001 {
			t.Fatalf("API_UNAVAILABLE = %d, want stable Operations web-entry code 66001", got)
		}
	}
}

func TestPublishedOperationsNumbersAreUniqueFiveDigitCodes(t *testing.T) {
	seen := make(map[int64]string, len(codeNumbers))
	for code, number := range codeNumbers {
		if number < 10_000 || number > 99_999 {
			t.Fatalf("%s has non-five-digit number %d", code, number)
		}
		if previous, exists := seen[number]; exists {
			t.Fatalf("%s and %s reuse error number %d", previous, code, number)
		}
		seen[number] = code
	}
}
