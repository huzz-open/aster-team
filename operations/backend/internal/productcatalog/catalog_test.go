package productcatalog

import (
	"reflect"
	"testing"
)

func TestResolveFeaturesRejectsUnknownDuplicatesAndEmpty(t *testing.T) {
	for _, selected := range [][]string{nil, {}, {"all"}, {"member", "unsupported"}, {"member", "member"}} {
		if _, err := ResolveFeatures(selected); err == nil {
			t.Errorf("accepted invalid capabilities %v", selected)
		}
	}
}

func TestResolveFeaturesKeepsExplicitSelectionAndStableOrder(t *testing.T) {
	for _, selected := range [][]string{{"member"}, {"runner", "gateway", "member"}} {
		before := append([]string(nil), selected...)
		resolved, err := ResolveFeatures(selected)
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(selected, before) {
			t.Fatal("mutated input")
		}
		if len(resolved) != len(selected) {
			t.Fatalf("unexpected capabilities: %v", resolved)
		}
		if len(selected) == 1 && resolved[0] != "member" {
			t.Fatal("expanded member into unrequested capabilities")
		}
		if len(selected) == 3 && !reflect.DeepEqual(resolved, []string{"gateway", "member", "runner"}) {
			t.Fatalf("unstable ordering %v", resolved)
		}
	}
}

func TestCallersCannotChangeCatalogOrTurnUnknownIntoKnown(t *testing.T) {
	operations := BusinessOperations()
	changedOperations := BusinessOperations()
	changedOperations[0].Requires[0] = "member"
	changedOperations[0].ID = "unknown"
	if !reflect.DeepEqual(BusinessOperations(), operations) {
		t.Fatal("operation requirement mutation escaped its caller")
	}
	original := Capabilities()
	changed := Capabilities()
	changed[0].ID = "all"
	changed[0].Requires = append(changed[0].Requires, "all")
	if !reflect.DeepEqual(Capabilities(), original) {
		t.Fatal("catalog mutation escaped its caller")
	}
	if _, ok := FindCapability("all"); ok {
		t.Fatal("unknown capability accepted")
	}
	if _, ok := FindQuota("all"); ok {
		t.Fatal("unknown quota accepted")
	}
}
