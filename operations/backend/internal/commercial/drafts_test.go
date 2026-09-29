package commercial

import (
	"bytes"
	"encoding/json"
	"strings"
	"testing"
)

func TestDraftRevisionAuthenticatesItsSelectionWithoutChangingPlan(t *testing.T) {
	draft := PlanDraftSnapshot{Schema: PlanDraftSchema, DraftID: "draft_1", Revision: 1, PlanID: "plan_1", ExpectedVersion: 0, Definition: testDefinition(t)}
	raw, err := draft.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	digest := ContentDigest(raw)
	stored, err := ParsePlanDraft(raw, digest)
	if err != nil {
		t.Fatal(err)
	}
	draft.Definition.Name = "changed unsaved input"
	draft.Definition.Entitlements.Features[0] = "unknown"
	if stored.Definition.Name == draft.Definition.Name {
		t.Fatal("draft retained mutable caller fields")
	}
	for name, mutate := range map[string]func(map[string]any){
		"missing":  func(v map[string]any) { delete(v, "expected_version") },
		"unknown":  func(v map[string]any) { v["published"] = true },
		"case":     func(v map[string]any) { v["Revision"] = v["revision"]; delete(v, "revision") },
		"overflow": func(v map[string]any) { v["expected_version"] = 4294967295 },
	} {
		t.Run(name, func(t *testing.T) {
			var v map[string]any
			if err := json.Unmarshal(raw, &v); err != nil {
				t.Fatal(err)
			}
			mutate(v)
			changed, err := canonical(v)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := ParsePlanDraft(changed, ContentDigest(changed)); err == nil {
				t.Fatal("invalid draft accepted even after digest recomputation")
			}
		})
	}
	if _, err := ParsePlanDraft(bytes.Replace(raw, []byte(`"revision":1`), []byte(`"revision":2`), 1), digest); err == nil {
		t.Fatal("draft revision edit ignored")
	}
	duplicate := []byte(strings.Replace(string(raw), `"revision":1`, `"revision":1,"revision":1`, 1))
	if _, err := ParsePlanDraft(duplicate, ContentDigest(duplicate)); err == nil {
		t.Fatal("duplicate field accepted")
	}
	if _, err := ParsePlanDraft(append(raw, '\n'), ContentDigest(append(raw, '\n'))); err == nil {
		t.Fatal("noncanonical stored snapshot accepted")
	}
}
