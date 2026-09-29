// Package strictjson validates exact object fields before Go's permissive JSON decoding.
package strictjson

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"unicode/utf8"
)

// Object requires every named field once with a non-null value. Call it for each
// nested wire struct too; it never treats case-folded field aliases as equivalent.
func Object(data []byte, target any, fields ...string) error {
	return ObjectWithin(data, target, 1<<20, fields...)
}

// ObjectWithin supports an explicitly bounded envelope around existing strict
// objects. Nested types retain their own smaller limits and all field checks.
func ObjectWithin(data []byte, target any, maximum int, fields ...string) error {
	if !utf8.Valid(data) {
		return fmt.Errorf("JSON is not valid UTF-8")
	}
	if maximum < 1 || maximum > 4<<20 || len(data) > maximum {
		return fmt.Errorf("JSON document is too large")
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err := consumeUniqueJSON(decoder, 0); err != nil {
		return err
	}
	if _, err := decoder.Token(); err != io.EOF {
		return fmt.Errorf("trailing JSON")
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(data, &object); err != nil {
		return err
	}
	if len(object) != len(fields) {
		return fmt.Errorf("missing or unknown JSON fields")
	}
	for _, field := range fields {
		if raw, exists := object[field]; !exists || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			return fmt.Errorf("missing JSON field %q", field)
		}
	}
	typed := json.NewDecoder(bytes.NewReader(data))
	typed.DisallowUnknownFields()
	return typed.Decode(target)
}

func consumeUniqueJSON(decoder *json.Decoder, depth int) error {
	if depth > 64 {
		return fmt.Errorf("JSON nesting is too deep")
	}
	token, err := decoder.Token()
	if err != nil {
		return err
	}
	delimiter, container := token.(json.Delim)
	if !container {
		return nil
	}
	switch delimiter {
	case '{':
		keys := map[string]bool{}
		for decoder.More() {
			token, err := decoder.Token()
			if err != nil {
				return err
			}
			key, ok := token.(string)
			if !ok || keys[key] {
				return fmt.Errorf("duplicate or invalid JSON field")
			}
			keys[key] = true
			if err := consumeUniqueJSON(decoder, depth+1); err != nil {
				return err
			}
		}
	case '[':
		for decoder.More() {
			if err := consumeUniqueJSON(decoder, depth+1); err != nil {
				return err
			}
		}
	default:
		return fmt.Errorf("unexpected JSON delimiter")
	}
	_, err = decoder.Token()
	return err
}
