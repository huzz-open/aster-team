package domain

import (
	"strings"
	"testing"
)

func TestValidSemanticVersion(t *testing.T) {
	t.Parallel()
	valid := []string{
		"0.0.0", "2.0.0", "1.0.0-alpha", "1.0.0-alpha.1", "1.0.0+20130313144700",
		"1.0.0-beta.2+exp.sha.5114f85",
	}
	invalid := []string{
		"", "v1.2.3", "1.2", "01.2.3", "1.02.3", "1.2.03", "1.0.0-", "1.0.0+",
		"1.0.0-alpha..1", "1.0.0-alpha.01", "1.0.0+build..1", "1.0.0\n", strings.Repeat("1", 65),
	}
	for _, value := range valid {
		if !ValidSemanticVersion(value) {
			t.Errorf("ValidSemanticVersion(%q) = false, want true", value)
		}
	}
	for _, value := range invalid {
		if ValidSemanticVersion(value) {
			t.Errorf("ValidSemanticVersion(%q) = true, want false", value)
		}
	}
}

func TestUpgradeSemanticVersionPrecedence(t *testing.T) {
	for _, pair := range [][2]string{{"2.0.2", "2.0.1"}, {"2.0.1", "2.0.1-rc.9"}, {"2.0.1-rc.10", "2.0.1-rc.2"}, {"2.0.1-beta", "2.0.1-99"}, {"2.0.1-alpha.1", "2.0.1-alpha"}} {
		if !IsNewerSemanticVersion(pair[0], pair[1]) || IsNewerSemanticVersion(pair[1], pair[0]) {
			t.Errorf("wrong precedence %v", pair)
		}
	}
	if IsNewerSemanticVersion("2.0.1+new", "2.0.1+old") || IsNewerSemanticVersion("invalid", "2.0.1") {
		t.Fatal("metadata or invalid version changed precedence")
	}
}
