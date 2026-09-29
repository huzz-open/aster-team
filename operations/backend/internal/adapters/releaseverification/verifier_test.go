package releaseverification

import (
	"archive/tar"
	"archive/zip"
	"bytes"
	"compress/gzip"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"debug/elf"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"path"
	"sort"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/config"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

type memoryArtifactSource struct {
	contents  []byte
	downloads *int
}

func (source memoryArtifactSource) DownloadArtifact(context.Context, int64) (io.ReadCloser, error) {
	if source.downloads != nil {
		(*source.downloads)++
	}
	return io.NopCloser(bytes.NewReader(source.contents)), nil
}

type memoryArtifactStore struct {
	stored []byte
	cache  []byte
}

func (*memoryArtifactStore) ImportInbox(context.Context, string, string) (ports.StoredArtifact, error) {
	return ports.StoredArtifact{}, errors.New("unexpected ImportInbox")
}
func (store *memoryArtifactStore) PutObject(_ context.Context, source io.Reader, expected string, maximum int64) (ports.StoredArtifact, error) {
	contents, err := io.ReadAll(io.LimitReader(source, maximum+1))
	if err != nil || int64(len(contents)) > maximum {
		return ports.StoredArtifact{}, errors.New("object too large")
	}
	digest := sha256.Sum256(contents)
	if hex.EncodeToString(digest[:]) != expected {
		return ports.StoredArtifact{}, errors.New("digest mismatch")
	}
	store.stored = contents
	return ports.StoredArtifact{ObjectKey: "objects/" + expected[:2] + "/" + expected, SHA256: expected, SizeBytes: int64(len(contents))}, nil
}
func (store *memoryArtifactStore) PutCachedObject(_ context.Context, source io.Reader, expected string, maximum int64) (ports.StoredArtifact, error) {
	contents, err := io.ReadAll(io.LimitReader(source, maximum+1))
	if err != nil || int64(len(contents)) > maximum {
		return ports.StoredArtifact{}, errors.New("cached object too large")
	}
	digest := sha256.Sum256(contents)
	if hex.EncodeToString(digest[:]) != expected {
		return ports.StoredArtifact{}, errors.New("cached object digest mismatch")
	}
	store.cache = append([]byte(nil), contents...)
	return ports.StoredArtifact{ObjectKey: "cache/" + expected[:2] + "/" + expected, SHA256: expected, SizeBytes: int64(len(contents))}, nil
}
func (*memoryArtifactStore) PutReceipt(context.Context, []byte) (ports.StoredArtifact, error) {
	return ports.StoredArtifact{}, errors.New("unexpected PutReceipt")
}
func (*memoryArtifactStore) OpenObject(context.Context, string, int64) (ports.ReadSeekCloser, int64, error) {
	return nil, 0, errors.New("unexpected OpenObject")
}
func (store *memoryArtifactStore) OpenCachedObject(_ context.Context, expected string, maximum int64) (ports.ReadSeekCloser, int64, error) {
	if len(store.cache) == 0 {
		return nil, 0, errors.New("cache miss")
	}
	digest := sha256.Sum256(store.cache)
	if hex.EncodeToString(digest[:]) != expected || int64(len(store.cache)) > maximum {
		return nil, 0, errors.New("cached object is invalid")
	}
	return &memoryReadSeekCloser{Reader: bytes.NewReader(store.cache)}, int64(len(store.cache)), nil
}
func (*memoryArtifactStore) ReadObject(context.Context, string, int64) ([]byte, error) {
	return nil, errors.New("unexpected ReadObject")
}

type memoryReadSeekCloser struct{ *bytes.Reader }

func (*memoryReadSeekCloser) Close() error { return nil }

func TestVerifierAcceptsIndependentlySignedExactRelease(t *testing.T) {
	fixture := newReleaseFixture(t, false)
	store := &memoryArtifactStore{}
	downloads := 0
	verifier, err := New(memoryArtifactSource{contents: fixture.zip, downloads: &downloads}, store, config.ReleaseVerification{
		TrustedKeysJSON: fixture.keyring, MaxArtifactBytes: 64 << 20, MaxExpandedBytes: 128 << 20,
	})
	if err != nil {
		t.Fatal(err)
	}
	verified, err := verifier.Verify(context.Background(), fixture.task, fixture.artifact)
	if err != nil {
		t.Fatal(err)
	}
	if verified.SignatureKeyID != "release-test-01" || verified.RuntimeLinkage != "musl-static" || verified.SHA256 == "" || len(store.stored) == 0 {
		t.Fatalf("unexpected verified release: %#v", verified)
	}
	if _, err := verifier.Verify(context.Background(), fixture.task, fixture.artifact); err != nil {
		t.Fatal(err)
	}
	if downloads != 1 {
		t.Fatalf("artifact downloads = %d, want one cached download", downloads)
	}
}

func TestVerifierAcceptsLegacyReleaseWithoutClientTools(t *testing.T) {
	fixture := newReleaseFixture(t, false, releaseFixtureChanges{beforeSigning: func(files map[string]releaseFixtureFile) {
		delete(files, "client-tools/asterctl/windows-x86_64/asterctl.exe")
	}})
	verifier, err := New(memoryArtifactSource{contents: fixture.zip}, &memoryArtifactStore{}, config.ReleaseVerification{
		TrustedKeysJSON: fixture.keyring, MaxArtifactBytes: 64 << 20, MaxExpandedBytes: 128 << 20,
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := verifier.Verify(context.Background(), fixture.task, fixture.artifact); err != nil {
		t.Fatalf("legacy release was rejected: %v", err)
	}
}

func TestVerifierRejectsPackageWithDifferentBundledFreeLicense(t *testing.T) {
	fixture := newReleaseFixture(t, false)
	fixture.task.FreeLicenseSHA256 = strings.Repeat("0", 64)
	assertFixtureRejected(t, fixture, "RELEASE_FREE_LICENSE_MISMATCH", "differs from the release task")
}

func TestVerifierRejectsUnsupportedSignedClientToolPaths(t *testing.T) {
	for _, name := range []string{
		"client-tools/other.exe",
		"client-tools/asterctl/windows-x86_64/other.exe",
		"client-tools/asterctl/windows-arm64/asterctl.exe",
		"client-tools/asterctl/windows-x86_64/operations.env",
		"client-tools/src/index.ts",
		"client-tools",
	} {
		t.Run(name, func(t *testing.T) {
			fixture := newReleaseFixture(t, false, releaseFixtureChanges{beforeSigning: func(files map[string]releaseFixtureFile) {
				file := files["client-tools/asterctl/windows-x86_64/asterctl.exe"]
				delete(files, "client-tools/asterctl/windows-x86_64/asterctl.exe")
				files[name] = file
			}})
			assertFixtureRejected(t, fixture, "RELEASE_PACKAGE_POLICY_FAILED", "client-tools")
		})
	}
}

func TestVerifierStillRejectsUnknownTopLevelAndSourceFiles(t *testing.T) {
	for _, name := range []string{"unexpected/tool.exe", "libexec/node_modules/tool.js", "admin/debug.map"} {
		t.Run(name, func(t *testing.T) {
			fixture := newReleaseFixture(t, false, releaseFixtureChanges{beforeSigning: func(files map[string]releaseFixtureFile) {
				files[name] = releaseFixtureFile{contents: []byte("not permitted")}
			}})
			assertFixtureRejected(t, fixture, "RELEASE_PACKAGE_POLICY_FAILED", "")
		})
	}
}

func TestVerifierRejectsTamperedOrUnsignedClientTool(t *testing.T) {
	t.Run("tampered signed bytes", func(t *testing.T) {
		fixture := newReleaseFixture(t, false, releaseFixtureChanges{afterSigning: func(files map[string]releaseFixtureFile) {
			files["client-tools/asterctl/windows-x86_64/asterctl.exe"] = releaseFixtureFile{contents: []byte("tampered")}
		}})
		assertFixtureRejected(t, fixture, "RELEASE_PACKAGE_POLICY_FAILED", "release manifest does not match client-tools/")
	})
	t.Run("absent from signed manifest", func(t *testing.T) {
		fixture := newReleaseFixture(t, false, releaseFixtureChanges{
			beforeSigning: func(files map[string]releaseFixtureFile) {
				delete(files, "client-tools/asterctl/windows-x86_64/asterctl.exe")
			},
			afterSigning: func(files map[string]releaseFixtureFile) {
				files["client-tools/asterctl/windows-x86_64/asterctl.exe"] = releaseFixtureFile{contents: []byte("unsigned")}
			},
		})
		assertFixtureRejected(t, fixture, "RELEASE_PACKAGE_POLICY_FAILED", "file trees differ")
	})
}

func assertFixtureRejected(t *testing.T, fixture releaseFixture, code, detail string) {
	t.Helper()
	store := &memoryArtifactStore{}
	verifier, err := New(memoryArtifactSource{contents: fixture.zip}, store, config.ReleaseVerification{
		TrustedKeysJSON: fixture.keyring, MaxArtifactBytes: 64 << 20, MaxExpandedBytes: 128 << 20,
	})
	if err != nil {
		t.Fatal(err)
	}
	_, err = verifier.Verify(context.Background(), fixture.task, fixture.artifact)
	var verificationError *ports.ReleaseArtifactVerificationError
	if !errors.As(err, &verificationError) || verificationError.Code != code || !strings.Contains(err.Error(), detail) {
		t.Fatalf("Verify() error = %v, want %s containing %q", err, code, detail)
	}
	if len(store.stored) != 0 {
		t.Fatal("rejected artifact was imported")
	}
}

func TestStaticELFInspectionRejectsDynamicInterpreter(t *testing.T) {
	if err := verifyStaticLinuxAMD64ELF(bytes.NewReader(fakeAMD64ELF())); err != nil {
		t.Fatalf("static ELF was rejected: %v", err)
	}
	if err := verifyStaticLinuxAMD64ELF(bytes.NewReader(fakeDynamicAMD64ELF())); err == nil {
		t.Fatal("ELF with PT_INTERP was accepted")
	}
}

func TestVerifierRejectsSignatureFromUntrustedKey(t *testing.T) {
	fixture := newReleaseFixture(t, true)
	verifier, err := New(memoryArtifactSource{contents: fixture.zip}, &memoryArtifactStore{}, config.ReleaseVerification{
		TrustedKeysJSON: fixture.keyring, MaxArtifactBytes: 64 << 20, MaxExpandedBytes: 128 << 20,
	})
	if err != nil {
		t.Fatal(err)
	}
	_, err = verifier.Verify(context.Background(), fixture.task, fixture.artifact)
	var verificationError *ports.ReleaseArtifactVerificationError
	if !errors.As(err, &verificationError) || verificationError.Code != "RELEASE_SIGNATURE_INVALID" {
		t.Fatalf("Verify() error = %v", err)
	}
}

func TestTrustedReleaseKeyringRejectsDuplicateJSONFields(t *testing.T) {
	_, err := parseTrustedKeys([]byte(`[{"key_id":"release-test-01","key_id":"release-test-02","public_key_spki":"invalid"}]`))
	if err == nil {
		t.Fatal("duplicated keyring field was accepted")
	}
}

type releaseFixture struct {
	zip      []byte
	keyring  string
	task     domain.ReleaseTask
	artifact domain.ReleaseTaskArtifact
}

type releaseFixtureFile struct {
	contents   []byte
	executable bool
}

type releaseFixtureChanges struct {
	platform      string
	beforeSigning func(map[string]releaseFixtureFile)
	afterSigning  func(map[string]releaseFixtureFile)
}

func newReleaseFixture(t *testing.T, signWithUntrustedKey bool, changes ...releaseFixtureChanges) releaseFixture {
	t.Helper()
	publicKey, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	_, otherPrivateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	der, err := x509.MarshalPKIXPublicKey(publicKey)
	if err != nil {
		t.Fatal(err)
	}
	keyringBytes, _ := json.Marshal([]trustedKeyEntry{{KeyID: "release-test-01", PublicKeySPKI: base64.RawURLEncoding.EncodeToString(der)}})
	version := "2.0.0"
	target := domain.ReleaseTarget{Platform: "linux", Architecture: "amd64"}
	for _, change := range changes {
		if change.platform != "" {
			target.Platform = change.platform
		}
	}
	policy, err := policyFor(target)
	if err != nil {
		t.Fatal(err)
	}
	files := map[string]releaseFixtureFile{
		"README.md":     {contents: []byte("install\n")},
		"SBOM.cdx.json": {contents: []byte(`{"$schema":"https://cyclonedx.org/schema/bom-1.6.schema.json","bomFormat":"CycloneDX","specVersion":"1.6","version":1,"metadata":{"component":{"type":"application","name":"aster-team-customer","version":"2.0.0"}},"components":[{}],"dependencies":[{}]}`)},
		"THIRD_PARTY_LICENSES/Caddy-Apache-2.0.txt": {contents: []byte("Apache License 2.0\n")},
		"VERSION":                    {contents: []byte(version + "\n")},
		"licenses/free-license.json": {contents: []byte(`{"signed":"free-license-fixture"}`)},
		"bin/aster-control":          {contents: fakeAMD64ELF(), executable: true},
		"bin/aster-runner":           {contents: fakeAMD64ELF(), executable: true},
		"bin/aster-team-cli":         {contents: fakeAMD64ELF(), executable: true},
		"init.sh":                    {contents: []byte("#!/bin/sh\n"), executable: true},
		"libexec/install.sh":         {contents: []byte("#!/bin/sh\n"), executable: true},
		"libexec/restore-backup.sh":  {contents: []byte("#!/bin/sh\n"), executable: true},
		"client-tools/asterctl/windows-x86_64/asterctl.exe": {contents: []byte("MZ signed Windows asterctl fixture")},
	}
	var sbom map[string]any
	if err := json.Unmarshal(files["SBOM.cdx.json"].contents, &sbom); err != nil {
		t.Fatal(err)
	}
	sbom["metadata"].(map[string]any)["properties"] = []map[string]string{{"name": "aster:target", "value": target.Platform + "-" + target.Architecture}, {"name": "aster:runtime", "value": policy.sbomRuntime}, {"name": "aster:rust-target", "value": policy.rustTarget}, {"name": "aster:storage", "value": "sqlcipher"}}
	sbomBytes, err := json.Marshal(sbom)
	if err != nil {
		t.Fatal(err)
	}
	files["SBOM.cdx.json"] = releaseFixtureFile{contents: sbomBytes}
	if target.Platform == "windows" {
		for _, name := range []string{"bin/aster-team-cli", "bin/aster-control", "bin/aster-runner", "init.sh", "libexec/install.sh", "libexec/restore-backup.sh"} {
			delete(files, name)
		}
		for name, executable := range policy.required {
			if _, ok := files[name]; !ok {
				files[name] = releaseFixtureFile{contents: []byte("Windows fixture\n"), executable: executable}
			}
		}
		for name, executable := range policy.binaries {
			files[name] = releaseFixtureFile{contents: fakeAMD64PE("KERNEL32.dll"), executable: executable}
		}
	}
	for _, change := range changes {
		if change.beforeSigning != nil {
			change.beforeSigning(files)
		}
	}
	fileNames := make([]string, 0, len(files))
	for name := range files {
		fileNames = append(fileNames, name)
	}
	sort.Strings(fileNames)
	claims := releaseClaims{Schema: releaseSchema, KeyID: "release-test-01", Product: releaseProduct, Version: version,
		Platform: target.Platform, Architecture: target.Architecture, Runtime: policy.runtime, CreatedAt: "2026-08-28T12:00:00.000Z"}
	for _, name := range fileNames {
		file := files[name]
		digest := sha256.Sum256(file.contents)
		claims.Files = append(claims.Files, releaseFile{Path: name, Size: uint64(len(file.contents)), SHA256: hex.EncodeToString(digest[:]), Executable: file.executable})
	}
	canonical, err := canonicalClaims(claims)
	if err != nil {
		t.Fatal(err)
	}
	signingKey := privateKey
	if signWithUntrustedKey {
		signingKey = otherPrivateKey
	}
	document := releaseDocument{releaseClaims: claims, Signature: base64.RawURLEncoding.EncodeToString(ed25519.Sign(signingKey, canonical))}
	manifest, err := json.MarshalIndent(document, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	manifest = append(manifest, '\n')
	files["RELEASE.json"] = releaseFixtureFile{contents: manifest}
	for _, change := range changes {
		if change.afterSigning != nil {
			change.afterSigning(files)
		}
	}

	root := strings.TrimSuffix(target.FileName(version), ".tar.gz")
	var archive bytes.Buffer
	gzipWriter := gzip.NewWriter(&archive)
	tarWriter := tar.NewWriter(gzipWriter)
	writeTarDirectory(t, tarWriter, root)
	directories := make(map[string]struct{})
	for name := range files {
		for directory := path.Dir(name); directory != "."; directory = path.Dir(directory) {
			directories[directory] = struct{}{}
		}
	}
	directoryNames := make([]string, 0, len(directories))
	for name := range directories {
		directoryNames = append(directoryNames, name)
	}
	sort.Strings(directoryNames)
	for _, name := range directoryNames {
		writeTarDirectory(t, tarWriter, root+"/"+name)
	}
	allNames := make([]string, 0, len(files))
	for name := range files {
		allNames = append(allNames, name)
	}
	sort.Strings(allNames)
	for _, name := range allNames {
		file := files[name]
		mode := int64(0o644)
		if file.executable {
			mode = 0o755
		}
		header := &tar.Header{Name: root + "/" + name, Mode: mode, Size: int64(len(file.contents)), Typeflag: tar.TypeReg, Format: tar.FormatUSTAR}
		if err := tarWriter.WriteHeader(header); err != nil {
			t.Fatal(err)
		}
		if _, err := tarWriter.Write(file.contents); err != nil {
			t.Fatal(err)
		}
	}
	if err := tarWriter.Close(); err != nil {
		t.Fatal(err)
	}
	if err := gzipWriter.Close(); err != nil {
		t.Fatal(err)
	}
	archiveDigest := sha256.Sum256(archive.Bytes())
	archiveName := root + ".tar.gz"
	checksum := hex.EncodeToString(archiveDigest[:]) + "  " + archiveName + "\n"
	var artifactZip bytes.Buffer
	zipWriter := zip.NewWriter(&artifactZip)
	for _, entry := range []struct {
		name     string
		contents []byte
	}{{archiveName, archive.Bytes()}, {archiveName + ".sha256", []byte(checksum)}} {
		writer, err := zipWriter.Create(entry.name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := writer.Write(entry.contents); err != nil {
			t.Fatal(err)
		}
	}
	if err := zipWriter.Close(); err != nil {
		t.Fatal(err)
	}
	zipDigest := sha256.Sum256(artifactZip.Bytes())
	githubDigest := hex.EncodeToString(zipDigest[:])
	now := time.Date(2026, 8, 28, 12, 0, 0, 0, time.UTC)
	freeLicenseDigest := sha256.Sum256(files["licenses/free-license.json"].contents)
	return releaseFixture{zip: artifactZip.Bytes(), keyring: string(keyringBytes),
		task: domain.ReleaseTask{ID: "release_task_1", Version: version, FreeDistributionID: "dist_fixture", FreeLicenseSHA256: hex.EncodeToString(freeLicenseDigest[:])},
		artifact: domain.ReleaseTaskArtifact{Platform: target.Platform, Architecture: target.Architecture, ID: "release_task_artifact_1", GitHubArtifactID: 1,
			Name: target.ArtifactName(version), GitHubDigestSHA256: &githubDigest, VerificationStatus: "pending", CreatedAt: now}}
}

func fakeAMD64ELF() []byte {
	header := make([]byte, 64)
	copy(header, []byte{0x7f, 'E', 'L', 'F', 2, 1, 1})
	binary.LittleEndian.PutUint16(header[16:18], uint16(2))
	binary.LittleEndian.PutUint16(header[18:20], uint16(62))
	binary.LittleEndian.PutUint32(header[20:24], uint32(1))
	binary.LittleEndian.PutUint16(header[52:54], uint16(64))
	binary.LittleEndian.PutUint16(header[54:56], uint16(56))
	binary.LittleEndian.PutUint16(header[58:60], uint16(64))
	return header
}

func fakeDynamicAMD64ELF() []byte {
	contents := append(fakeAMD64ELF(), make([]byte, 56)...)
	binary.LittleEndian.PutUint64(contents[32:40], uint64(64))
	binary.LittleEndian.PutUint16(contents[56:58], uint16(1))
	binary.LittleEndian.PutUint32(contents[64:68], uint32(elf.PT_INTERP))
	return contents
}

func writeTarDirectory(t *testing.T, writer *tar.Writer, name string) {
	t.Helper()
	if err := writer.WriteHeader(&tar.Header{Name: strings.TrimSuffix(name, "/") + "/", Mode: 0o755, Typeflag: tar.TypeDir, Format: tar.FormatUSTAR}); err != nil {
		t.Fatal(err)
	}
}

func TestStorageProfileMatchesTheVerifiedPackagePlatform(t *testing.T) {
	for _, platform := range []string{"linux", "windows"} {
		policy, err := policyFor(domain.ReleaseTarget{Platform: platform, Architecture: "amd64"})
		if err != nil {
			t.Fatal(err)
		}
		if !supportedStorageProfile("sqlcipher", policy) {
			t.Fatal("legacy signed packages must remain valid")
		}
		if supportedStorageProfile("sqlcipher,mariadb", policy) != (platform == "linux") {
			t.Fatal("dual storage must be confined to Linux amd64")
		}
		for _, unsupported := range []string{"mysql", "mariadb", "sqlcipher,mysql", "sqlcipher,mariadb,unknown"} {
			if supportedStorageProfile(unsupported, policy) {
				t.Fatalf("accepted unknown storage profile %s", unsupported)
			}
		}
	}
}
