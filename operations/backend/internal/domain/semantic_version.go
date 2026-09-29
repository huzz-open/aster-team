package domain

import (
	"regexp"
	"strings"
)

const MaximumSemanticVersionLength = 64

var semanticVersionPattern = regexp.MustCompile(`^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-((0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(\+([0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*))?$`)

// ValidSemanticVersion validates the complete SemVer 2.0.0 grammar used by release inputs.
func ValidSemanticVersion(value string) bool {
	return len(value) <= MaximumSemanticVersionLength && semanticVersionPattern.MatchString(value)
}

// IsNewerSemanticVersion compares precedence, ignoring build metadata.
func IsNewerSemanticVersion(candidate, current string) bool {
	if !ValidSemanticVersion(candidate) || !ValidSemanticVersion(current) {
		return false
	}
	a, _, _ := strings.Cut(candidate, "+")
	b, _, _ := strings.Cut(current, "+")
	ac, ap, _ := strings.Cut(a, "-")
	bc, bp, _ := strings.Cut(b, "-")
	aa, bb := strings.Split(ac, "."), strings.Split(bc, ".")
	for i := range aa {
		if result := compareVersionNumber(aa[i], bb[i]); result != 0 {
			return result > 0
		}
	}
	if ap == bp {
		return false
	}
	if ap == "" {
		return true
	}
	if bp == "" {
		return false
	}
	aa, bb = strings.Split(ap, "."), strings.Split(bp, ".")
	for i := 0; i < len(aa) && i < len(bb); i++ {
		if aa[i] == bb[i] {
			continue
		}
		an, bn := numericVersionPart(aa[i]), numericVersionPart(bb[i])
		if an && bn {
			return compareVersionNumber(aa[i], bb[i]) > 0
		}
		if an != bn {
			return !an
		}
		return aa[i] > bb[i]
	}
	return len(aa) > len(bb)
}

func numericVersionPart(value string) bool {
	for _, ch := range value {
		if ch < '0' || ch > '9' {
			return false
		}
	}
	return true
}

func compareVersionNumber(a, b string) int {
	if len(a) < len(b) {
		return -1
	}
	if len(a) > len(b) {
		return 1
	}
	return strings.Compare(a, b)
}
