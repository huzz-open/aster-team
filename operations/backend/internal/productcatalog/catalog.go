package productcatalog

import (
	"fmt"
	"sort"
)

// ResolveFeatures validates a new plan selection and freezes its dependency closure.
// It never interprets an unknown feature, edition or an empty selection as all features.
func ResolveFeatures(selected []string) ([]string, error) {
	if len(selected) == 0 || len(selected) > 64 {
		return nil, fmt.Errorf("select between 1 and 64 supported capabilities")
	}
	seen := make(map[CapabilityID]bool, len(selected))
	for _, value := range selected {
		id := CapabilityID(value)
		if _, ok := FindCapability(id); !ok {
			return nil, fmt.Errorf("unknown capability %q", value)
		}
		if seen[id] {
			return nil, fmt.Errorf("duplicate capability %q", value)
		}
		seen[id] = true
	}
	var include func(CapabilityID)
	include = func(id CapabilityID) {
		entry, _ := FindCapability(id)
		for _, dependency := range entry.Requires {
			if !seen[dependency] {
				seen[dependency] = true
				include(dependency)
			}
		}
	}
	for _, value := range selected {
		include(CapabilityID(value))
	}
	result := make([]string, 0, len(seen))
	for id := range seen {
		result = append(result, string(id))
	}
	sort.Strings(result)
	return result, nil
}
