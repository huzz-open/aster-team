package strictjson

import (
	"strings"
	"testing"
)

func TestObjectRejectsAmbiguousWireInput(t *testing.T) {
	var value struct {
		Value uint32 `json:"value"`
	}
	for _, raw := range []string{`null`, `{}`, `{"value":null}`, `{"Value":1}`, `{"value":1,"value":1}`, `{"value":1,"VALUE":1}`, `{"value":1,"extra":0}`, `{"value":1} false`, `{"value":1} garbage`, `{"value":1.0}`, `{"value":-1}`, `{"value":4294967296}`, "{\"value\":1,\"" + string([]byte{0xff}) + "\":0}", strings.Repeat("[", 66) + "0" + strings.Repeat("]", 66), strings.Repeat(" ", 1<<20) + `{"value":1}`} {
		if err := Object([]byte(raw), &value, "value"); err == nil {
			t.Fatalf("accepted invalid input %.150s", raw)
		}
	}
	for _, raw := range []string{`{"value":0}`, ` {"value":4294967295} `} {
		if err := Object([]byte(raw), &value, "value"); err != nil {
			t.Fatal(err)
		}
	}
}
