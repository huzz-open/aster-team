package signing

import (
	"encoding/base64"
	"strings"
	"testing"
)

func TestCustomerReferenceIsStableAndOpaque(t *testing.T) {
	source, err := NewHMACCustomerReferenceSource(base64.RawURLEncoding.EncodeToString([]byte(strings.Repeat("s", 32))))
	if err != nil {
		t.Fatal(err)
	}
	first, err := source.Reference("cust_local_customer_name")
	if err != nil {
		t.Fatal(err)
	}
	second, _ := source.Reference("cust_local_customer_name")
	other, _ := source.Reference("cust_other")
	if first != second || first == other || strings.Contains(first, "local_customer_name") || !strings.HasPrefix(first, "customer_") {
		t.Fatalf("customer reference is not stable and opaque: %q %q %q", first, second, other)
	}
}
