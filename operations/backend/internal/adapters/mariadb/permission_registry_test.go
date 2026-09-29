package mariadb

import (
	"io/fs"
	"regexp"
	"slices"
	"sort"
	"strings"
	"testing"

	"aster.local/team/operations/backend/internal/application"
	operationsschema "aster.local/team/operations/backend/schema"
)

var (
	permissionConstraintAddPattern  = regexp.MustCompile(`(?i)CONSTRAINT\s+([0-9A-Za-z_]+)\s+CHECK\s*\(`)
	permissionConstraintDropPattern = regexp.MustCompile(`(?i)DROP\s+CONSTRAINT\s+(?:IF\s+EXISTS\s+)?([0-9A-Za-z_]+)`)
	permissionCodeLiteralPattern    = regexp.MustCompile(`'([^']+)'`)
)

type permissionConstraintEvent struct {
	position int
	drop     bool
	name     string
	body     string
}

// MariaDB enforces every CHECK constraint on operator_permissions.permission_code
// at once, so a code one registry rejects can never be inserted even when another
// registry allows it. This pins the property that bootstrapped administrator
// permissions survive the registry left behind by the whole migration chain.
func TestOperationsSchemaKeepsOnePermissionRegistry(t *testing.T) {
	paths, err := schemaFilePaths(operationsschema.Files)
	if err != nil {
		t.Fatal(err)
	}
	statements := make([]string, 0, 64)
	for _, path := range paths {
		contents, err := fs.ReadFile(operationsschema.Files, path)
		if err != nil {
			t.Fatalf("read schema file %s: %v", path, err)
		}
		statements = append(statements, splitStatements(string(canonicalMigrationContents(contents)))...)
	}
	active := replayPermissionConstraints(t, statements)
	if len(active) == 0 {
		t.Fatal("no CHECK constraint restricts operator_permissions.permission_code once every migration has been applied")
	}
	names := make([]string, 0, len(active))
	for name := range active {
		names = append(names, name)
	}
	sort.Strings(names)
	required := append(application.BootstrapReleasePermissions(), application.BootstrapCommercialPermissions()...)
	for _, name := range names {
		for _, permission := range required {
			if !slices.Contains(active[name], permission) {
				t.Errorf("constraint %s rejects %s, so the administrator bootstrap insert fails with ERROR 4025 (active constraints: %s)",
					name, permission, strings.Join(names, ", "))
			}
		}
	}
}

func TestReplayPermissionConstraintsDetectsOverlappingRegistries(t *testing.T) {
	// Shape of 202609060002 through 202609081100: a later change set re-added
	// chk_operator_permissions_release without dropping chk_operator_permissions_registry.
	statements := splitStatements(`ALTER TABLE operator_permissions
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_release,
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_registry,
    ADD CONSTRAINT chk_operator_permissions_registry CHECK (permission_code IN ('release.read'));
ALTER TABLE operator_permissions
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_release,
    ADD CONSTRAINT chk_operator_permissions_release CHECK (permission_code IN ('release.read', 'release.environment.write'));
`)
	active := replayPermissionConstraints(t, statements)
	if len(active) != 2 {
		t.Fatalf("replayed schema must keep both overlapping constraints, got %d", len(active))
	}
	if slices.Contains(active["chk_operator_permissions_registry"], "release.environment.write") {
		t.Fatal("release.environment.write must come from the release registry only")
	}
}

// replayPermissionConstraints applies schema statements in migration order and
// reports every CHECK constraint that still restricts
// operator_permissions.permission_code together with the codes it accepts.
func replayPermissionConstraints(t *testing.T, statements []string) map[string][]string {
	t.Helper()
	active := map[string][]string{}
	for _, statement := range statements {
		for _, event := range permissionConstraintEvents(statement) {
			if event.drop {
				delete(active, event.name)
				continue
			}
			codes := make([]string, 0, 8)
			for _, literal := range permissionCodeLiteralPattern.FindAllStringSubmatch(event.body, -1) {
				codes = append(codes, literal[1])
			}
			if len(codes) == 0 {
				t.Fatalf("permission constraint %s declares no permission code", event.name)
			}
			active[event.name] = codes
		}
	}
	return active
}

func permissionConstraintEvents(statement string) []permissionConstraintEvent {
	events := make([]permissionConstraintEvent, 0, 4)
	for _, match := range permissionConstraintDropPattern.FindAllStringSubmatchIndex(statement, -1) {
		events = append(events, permissionConstraintEvent{
			position: match[0],
			drop:     true,
			name:     statement[match[2]:match[3]],
		})
	}
	for _, match := range permissionConstraintAddPattern.FindAllStringSubmatchIndex(statement, -1) {
		body, ok := balancedParenthesisBody(statement[match[1]:])
		if !ok || !strings.Contains(body, "permission_code") {
			continue
		}
		events = append(events, permissionConstraintEvent{
			position: match[0],
			name:     statement[match[2]:match[3]],
			body:     body,
		})
	}
	sort.SliceStable(events, func(first, second int) bool { return events[first].position < events[second].position })
	return events
}

// balancedParenthesisBody returns the text up to the parenthesis that closes the
// group the caller already opened.
func balancedParenthesisBody(rest string) (string, bool) {
	depth := 1
	for index, character := range rest {
		switch character {
		case '(':
			depth++
		case ')':
			depth--
			if depth == 0 {
				return rest[:index], true
			}
		}
	}
	return "", false
}
