package releaseverification

import (
	"archive/tar"
	"archive/zip"
	"bytes"
	"compress/gzip"
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"crypto/x509"
	"debug/elf"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path"
	"regexp"
	"sort"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/config"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

const (
	releaseSchema       = "aster.release-manifest.v1"
	releaseProduct      = "aster-team"
	maximumManifestSize = 2 << 20
	maximumSBOMSize     = 16 << 20
	maximumChecksumSize = 512
)

var (
	releaseVersionPattern = regexp.MustCompile(`^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$`)
	digestPattern         = regexp.MustCompile(`^[0-9a-f]{64}$`)
	identifierPattern     = regexp.MustCompile(`^[A-Za-z0-9_.:-]{3,128}$`)
)

type Verifier struct {
	source           ports.ReleaseArtifactSource
	store            ports.ArtifactStore
	trustedKeys      map[string]ed25519.PublicKey
	maxArtifactBytes int64
	maxExpandedBytes int64
}

type trustedKeyEntry struct {
	KeyID         string `json:"key_id"`
	PublicKeySPKI string `json:"public_key_spki"`
}

type releaseFile struct {
	Path       string `json:"path"`
	Size       uint64 `json:"size"`
	SHA256     string `json:"sha256"`
	Executable bool   `json:"executable"`
}

type releaseClaims struct {
	Schema       string        `json:"schema"`
	KeyID        string        `json:"key_id"`
	Product      string        `json:"product"`
	Version      string        `json:"version"`
	Platform     string        `json:"platform"`
	Architecture string        `json:"architecture"`
	Runtime      string        `json:"runtime"`
	CreatedAt    string        `json:"created_at"`
	Files        []releaseFile `json:"files"`
}

type releaseDocument struct {
	releaseClaims
	Signature string `json:"signature"`
}

type actualFile struct {
	size       uint64
	sha256     string
	executable bool
}

type inspectedArchive struct {
	manifestSHA256 string
	signatureKeyID string
	runtimeLinkage string
}

func New(source ports.ReleaseArtifactSource, store ports.ArtifactStore, cfg config.ReleaseVerification) (*Verifier, error) {
	if source == nil || store == nil || cfg.MaxArtifactBytes < 1 || cfg.MaxExpandedBytes < cfg.MaxArtifactBytes {
		return nil, errors.New("release artifact verifier configuration is invalid")
	}
	keys, err := parseTrustedKeys([]byte(cfg.TrustedKeysJSON))
	if err != nil {
		return nil, err
	}
	return &Verifier{source: source, store: store, trustedKeys: keys, maxArtifactBytes: cfg.MaxArtifactBytes, maxExpandedBytes: cfg.MaxExpandedBytes}, nil
}

func (verifier *Verifier) Verify(ctx context.Context, task domain.ReleaseTask, artifact domain.ReleaseTaskArtifact) (ports.VerifiedReleaseArtifact, error) {
	policy, err := policyFor(artifact.Target())
	if err != nil {
		return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", err)
	}
	expectedArtifactName := policy.target.ArtifactName(task.Version)
	if artifact.Name != expectedArtifactName || artifact.VerificationStatus == "unavailable" || artifact.GitHubArtifactID <= 0 {
		return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_ARTIFACT_UNAVAILABLE", errors.New("the expected GitHub artifact is unavailable"))
	}
	if artifact.GitHubDigestSHA256 == nil || !digestPattern.MatchString(*artifact.GitHubDigestSHA256) {
		return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_ARTIFACT_HASH_MISMATCH", errors.New("GitHub artifact digest is missing or invalid"))
	}

	actualGitHubDigest := *artifact.GitHubDigestSHA256
	zipFile, zipSize, err := verifier.openCachedGitHubArtifact(ctx, artifact.GitHubArtifactID, actualGitHubDigest)
	if err != nil {
		return ports.VerifiedReleaseArtifact{}, err
	}
	defer zipFile.Close()

	archiveName := policy.target.FileName(task.Version)
	checksumName := archiveName + ".sha256"
	zipReader, err := zip.NewReader(zipFile, zipSize)
	if err != nil {
		return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("GitHub artifact is not a valid ZIP archive"))
	}
	entries := make(map[string]*zip.File, len(zipReader.File))
	var expandedZipBytes uint64
	for _, entry := range zipReader.File {
		if entry.FileInfo().IsDir() || entry.Mode()&os.ModeType != 0 || entry.Name != path.Base(entry.Name) || strings.ContainsAny(entry.Name, `/\`) {
			return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("GitHub artifact contains an unsafe path"))
		}
		if entry.Method != zip.Store && entry.Method != zip.Deflate {
			return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("GitHub artifact uses an unsupported compression method"))
		}
		if _, duplicate := entries[entry.Name]; duplicate {
			return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("GitHub artifact contains a duplicated file"))
		}
		maximumZIPBytes := uint64(verifier.maxArtifactBytes) + maximumChecksumSize
		if entry.UncompressedSize64 > maximumZIPBytes-expandedZipBytes {
			return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("GitHub artifact expands beyond the configured size limit"))
		}
		expandedZipBytes += entry.UncompressedSize64
		entries[entry.Name] = entry
	}
	if len(entries) != 2 || entries[archiveName] == nil || entries[checksumName] == nil {
		return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_ARTIFACT_UNAVAILABLE", errors.New("GitHub artifact must contain exactly the release archive and checksum"))
	}
	checksum, err := readZipEntry(entries[checksumName], maximumChecksumSize)
	if err != nil {
		return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_ARTIFACT_HASH_MISMATCH", err)
	}

	archiveFile, err := os.CreateTemp("", "aster-release-archive-*.tar.gz")
	if err != nil {
		return ports.VerifiedReleaseArtifact{}, err
	}
	archivePath := archiveFile.Name()
	defer os.Remove(archivePath)
	archiveSource, err := entries[archiveName].Open()
	if err != nil {
		archiveFile.Close()
		return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release archive cannot be opened"))
	}
	archiveDigest := sha256.New()
	archiveBytes, copyErr := io.Copy(io.MultiWriter(archiveFile, archiveDigest), io.LimitReader(archiveSource, verifier.maxArtifactBytes+1))
	sourceCloseErr := archiveSource.Close()
	if copyErr != nil || sourceCloseErr != nil || archiveBytes > verifier.maxArtifactBytes {
		archiveFile.Close()
		return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release archive is unreadable or too large"))
	}
	archiveSHA256 := hex.EncodeToString(archiveDigest.Sum(nil))
	if string(checksum) != archiveSHA256+"  "+archiveName+"\n" {
		archiveFile.Close()
		return ports.VerifiedReleaseArtifact{}, invalid("RELEASE_ARTIFACT_HASH_MISMATCH", errors.New("release archive checksum file does not match the archive"))
	}
	if _, err := archiveFile.Seek(0, io.SeekStart); err != nil {
		archiveFile.Close()
		return ports.VerifiedReleaseArtifact{}, err
	}
	inspected, err := verifier.inspectArchive(archiveFile, task, policy)
	if err != nil {
		archiveFile.Close()
		return ports.VerifiedReleaseArtifact{}, err
	}
	if _, err := archiveFile.Seek(0, io.SeekStart); err != nil {
		archiveFile.Close()
		return ports.VerifiedReleaseArtifact{}, err
	}
	stored, err := verifier.store.PutObject(ctx, archiveFile, archiveSHA256, verifier.maxArtifactBytes)
	archiveCloseErr := archiveFile.Close()
	if err != nil {
		return ports.VerifiedReleaseArtifact{}, fmt.Errorf("store verified release archive: %w", err)
	}
	if archiveCloseErr != nil {
		return ports.VerifiedReleaseArtifact{}, archiveCloseErr
	}
	return ports.VerifiedReleaseArtifact{ObjectKey: stored.ObjectKey, SHA256: stored.SHA256, SizeBytes: stored.SizeBytes,
		ManifestSHA256: inspected.manifestSHA256, SignatureKeyID: inspected.signatureKeyID,
		RuntimeLinkage: inspected.runtimeLinkage, SourceArtifactID: artifact.ID,
		GitHubDigestSHA256: actualGitHubDigest}, nil
}

func (verifier *Verifier) openCachedGitHubArtifact(ctx context.Context, artifactID int64, expectedSHA256 string) (ports.ReadSeekCloser, int64, error) {
	opened, size, err := verifier.store.OpenCachedObject(ctx, expectedSHA256, verifier.maxArtifactBytes)
	if err == nil {
		return opened, size, nil
	}
	response, err := verifier.source.DownloadArtifact(ctx, artifactID)
	if err != nil {
		return nil, 0, fmt.Errorf("download GitHub artifact: %w", err)
	}
	stored, putErr := verifier.store.PutCachedObject(ctx, response, expectedSHA256, verifier.maxArtifactBytes)
	closeErr := response.Close()
	if putErr != nil {
		return nil, 0, invalid("RELEASE_ARTIFACT_HASH_MISMATCH", fmt.Errorf("cache downloaded GitHub artifact: %w", putErr))
	}
	if closeErr != nil {
		return nil, 0, fmt.Errorf("close downloaded GitHub artifact: %w", closeErr)
	}
	opened, size, err = verifier.store.OpenCachedObject(ctx, expectedSHA256, verifier.maxArtifactBytes)
	if err != nil {
		return nil, 0, fmt.Errorf("open cached GitHub artifact %s: %w", stored.ObjectKey, err)
	}
	return opened, size, nil
}

func (verifier *Verifier) inspectArchive(archive io.Reader, task domain.ReleaseTask, policy platformPolicy) (inspectedArchive, error) {
	gzipReader, err := gzip.NewReader(archive)
	if err != nil {
		return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release archive is not valid gzip"))
	}
	defer gzipReader.Close()
	tape := tar.NewReader(gzipReader)
	root := strings.TrimSuffix(policy.target.FileName(task.Version), ".tar.gz")
	actual := make(map[string]actualFile)
	seenEntries := make(map[string]struct{})
	seenDirectories := make(map[string]struct{})
	staticBinaryCount := 0
	var manifestBytes, versionBytes, sbomBytes []byte
	var expanded int64
	for {
		header, err := tape.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release tar stream is invalid"))
		}
		if header.Format != tar.FormatUSTAR || header.PAXRecords != nil || header.Linkname != "" || header.Size < 0 {
			return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release tar entry format is unsafe"))
		}
		entryName := header.Name
		if header.Typeflag == tar.TypeDir {
			entryName = strings.TrimSuffix(entryName, "/")
		}
		if !safeArchivePath(entryName) || (entryName != root && !strings.HasPrefix(entryName, root+"/")) {
			return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release tar entry escapes its versioned root"))
		}
		if _, duplicate := seenEntries[entryName]; duplicate {
			return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release tar contains a duplicated entry"))
		}
		seenEntries[entryName] = struct{}{}
		if header.Typeflag == tar.TypeDir {
			if header.Mode != 0o755 {
				return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release directory permissions are invalid"))
			}
			seenDirectories[entryName] = struct{}{}
			continue
		}
		if header.Typeflag != tar.TypeReg || entryName == root {
			return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release tar contains a non-regular file"))
		}
		if header.Size > verifier.maxExpandedBytes-expanded {
			return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release archive expands beyond the configured size limit"))
		}
		expanded += header.Size
		relative := strings.TrimPrefix(entryName, root+"/")
		if !safeReleasePath(relative) {
			return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", fmt.Errorf("release file path is invalid: %s", relative))
		}
		executable := header.Mode&0o111 != 0
		if header.Mode != 0o644 && header.Mode != 0o755 {
			return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", fmt.Errorf("release file permissions are invalid: %s", relative))
		}
		digest := sha256.New()
		writers := []io.Writer{digest}
		var capture *bytes.Buffer
		switch relative {
		case "RELEASE.json":
			if header.Size > maximumManifestSize {
				return inspectedArchive{}, invalid("RELEASE_MANIFEST_INVALID", errors.New("release manifest is too large"))
			}
			capture = &bytes.Buffer{}
		case "VERSION":
			if header.Size > 128 {
				return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("VERSION is too large"))
			}
			capture = &bytes.Buffer{}
		case "SBOM.cdx.json":
			if header.Size > maximumSBOMSize {
				return inspectedArchive{}, invalid("RELEASE_SBOM_INVALID", errors.New("SBOM is too large"))
			}
			capture = &bytes.Buffer{}
		}
		var binaryFile *os.File
		if _, ok := policy.binaries[relative]; ok {
			binaryFile, err = os.CreateTemp("", "aster-release-binary-*")
			if err != nil {
				return inspectedArchive{}, err
			}
			writers = append(writers, binaryFile)
		}
		if capture != nil {
			writers = append(writers, capture)
		}
		copied, err := io.Copy(io.MultiWriter(writers...), tape)
		if err != nil || copied != header.Size {
			if binaryFile != nil {
				binaryPath := binaryFile.Name()
				binaryFile.Close()
				os.Remove(binaryPath)
			}
			return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", fmt.Errorf("release file is truncated: %s", relative))
		}
		if capture != nil {
			switch relative {
			case "RELEASE.json":
				manifestBytes = capture.Bytes()
			case "VERSION":
				versionBytes = capture.Bytes()
			case "SBOM.cdx.json":
				sbomBytes = capture.Bytes()
			}
		}
		if binaryFile != nil {
			binaryPath := binaryFile.Name()
			if _, err := binaryFile.Seek(0, io.SeekStart); err != nil {
				binaryFile.Close()
				os.Remove(binaryPath)
				return inspectedArchive{}, err
			}
			inspectionErr := policy.inspectBinary(binaryFile)
			closeErr := binaryFile.Close()
			removeErr := os.Remove(binaryPath)
			if inspectionErr != nil {
				return inspectedArchive{}, invalid("RELEASE_ABI_INCOMPATIBLE", fmt.Errorf("%s: %w", relative, inspectionErr))
			}
			if closeErr != nil || removeErr != nil {
				return inspectedArchive{}, errors.New("clean up temporary release binary")
			}
			staticBinaryCount++
		}
		if relative != "RELEASE.json" {
			actual[relative] = actualFile{size: uint64(header.Size), sha256: hex.EncodeToString(digest.Sum(nil)), executable: executable}
		}
	}
	if len(manifestBytes) == 0 || len(versionBytes) == 0 || len(sbomBytes) == 0 || staticBinaryCount != len(policy.binaries) {
		return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release archive is missing a required signed file"))
	}
	if err := verifyExactDirectories(root, actual, seenDirectories); err != nil {
		return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", err)
	}
	if string(versionBytes) != task.Version+"\n" {
		return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("VERSION does not match the release task"))
	}
	document, err := verifier.verifyManifest(manifestBytes, task, actual, policy)
	if err != nil {
		return inspectedArchive{}, err
	}
	if err := verifyPackagePolicy(actual, policy); err != nil {
		return inspectedArchive{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", err)
	}
	if err := verifyBundledFreeLicense(actual, task); err != nil {
		return inspectedArchive{}, invalid("RELEASE_FREE_LICENSE_MISMATCH", err)
	}
	if err := verifySBOM(sbomBytes, task.Version, policy); err != nil {
		return inspectedArchive{}, invalid("RELEASE_SBOM_INVALID", err)
	}
	manifestDigest := sha256.Sum256(manifestBytes)
	return inspectedArchive{manifestSHA256: hex.EncodeToString(manifestDigest[:]), signatureKeyID: document.KeyID, runtimeLinkage: policy.runtime}, nil
}

func (verifier *Verifier) verifyManifest(contents []byte, task domain.ReleaseTask, actual map[string]actualFile, policy platformPolicy) (releaseDocument, error) {
	if err := rejectDuplicateJSONKeys(contents); err != nil {
		return releaseDocument{}, invalid("RELEASE_MANIFEST_INVALID", err)
	}
	var document releaseDocument
	decoder := json.NewDecoder(bytes.NewReader(contents))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&document); err != nil {
		return releaseDocument{}, invalid("RELEASE_MANIFEST_INVALID", errors.New("release manifest JSON is invalid"))
	}
	if err := requireJSONEOF(decoder); err != nil {
		return releaseDocument{}, invalid("RELEASE_MANIFEST_INVALID", err)
	}
	if document.Schema != releaseSchema || document.Product != releaseProduct || document.Version != task.Version ||
		document.Platform != policy.target.Platform || document.Architecture != policy.target.Architecture || document.Runtime != policy.runtime || !identifierPattern.MatchString(document.KeyID) ||
		!exactReleaseTime(document.CreatedAt) || !releaseVersionPattern.MatchString(document.Version) || len(document.Files) == 0 || len(document.Files) > 10_000 {
		return releaseDocument{}, invalid("RELEASE_MANIFEST_INVALID", errors.New("release manifest claims are invalid"))
	}
	expected := make(map[string]releaseFile, len(document.Files))
	previous := ""
	for index, file := range document.Files {
		if !safeReleasePath(file.Path) || file.Path == "RELEASE.json" || !digestPattern.MatchString(file.SHA256) ||
			(index > 0 && previous >= file.Path) {
			return releaseDocument{}, invalid("RELEASE_MANIFEST_INVALID", errors.New("release manifest file list is invalid"))
		}
		previous = file.Path
		expected[file.Path] = file
	}
	if len(expected) != len(actual) {
		return releaseDocument{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", errors.New("release manifest and archive file trees differ"))
	}
	for name, file := range expected {
		observed, ok := actual[name]
		if !ok || observed.size != file.Size || observed.sha256 != file.SHA256 || observed.executable != file.Executable {
			return releaseDocument{}, invalid("RELEASE_PACKAGE_POLICY_FAILED", fmt.Errorf("release manifest does not match %s", name))
		}
	}
	key := verifier.trustedKeys[document.KeyID]
	if key == nil {
		return releaseDocument{}, invalid("RELEASE_SIGNATURE_INVALID", errors.New("release manifest key ID is not trusted by Operations"))
	}
	signature, err := base64.RawURLEncoding.DecodeString(document.Signature)
	if err != nil || len(signature) != ed25519.SignatureSize {
		return releaseDocument{}, invalid("RELEASE_SIGNATURE_INVALID", errors.New("release manifest signature encoding is invalid"))
	}
	canonical, err := canonicalClaims(document.releaseClaims)
	if err != nil || !ed25519.Verify(key, canonical, signature) {
		return releaseDocument{}, invalid("RELEASE_SIGNATURE_INVALID", errors.New("release manifest signature is invalid"))
	}
	return document, nil
}

func parseTrustedKeys(contents []byte) (map[string]ed25519.PublicKey, error) {
	if err := rejectDuplicateJSONKeys(contents); err != nil {
		return nil, fmt.Errorf("parse Operations trusted release keys: %w", err)
	}
	var entries []trustedKeyEntry
	decoder := json.NewDecoder(bytes.NewReader(contents))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&entries); err != nil || requireJSONEOF(decoder) != nil || len(entries) < 1 || len(entries) > 8 {
		return nil, errors.New("ASTER_OPERATIONS_RELEASE_TRUSTED_KEYS_JSON must contain one to eight strict key entries")
	}
	keys := make(map[string]ed25519.PublicKey, len(entries))
	publicKeys := make(map[string]struct{}, len(entries))
	for _, entry := range entries {
		if !identifierPattern.MatchString(entry.KeyID) || keys[entry.KeyID] != nil {
			return nil, errors.New("Operations trusted release key ID is invalid or duplicated")
		}
		der, err := base64.RawURLEncoding.DecodeString(entry.PublicKeySPKI)
		if err != nil || base64.RawURLEncoding.EncodeToString(der) != entry.PublicKeySPKI {
			return nil, errors.New("Operations trusted release public key is not canonical base64url")
		}
		parsed, err := x509.ParsePKIXPublicKey(der)
		key, ok := parsed.(ed25519.PublicKey)
		if err != nil || !ok || len(key) != ed25519.PublicKeySize {
			return nil, errors.New("Operations trusted release public key is not Ed25519 SPKI")
		}
		fingerprint := string(key)
		if _, duplicate := publicKeys[fingerprint]; duplicate {
			return nil, errors.New("Operations trusted release public key is duplicated")
		}
		publicKeys[fingerprint] = struct{}{}
		keys[entry.KeyID] = append(ed25519.PublicKey(nil), key...)
	}
	return keys, nil
}

func canonicalClaims(claims releaseClaims) ([]byte, error) {
	encoded, err := json.Marshal(claims)
	if err != nil {
		return nil, err
	}
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	decoder.UseNumber()
	var value any
	if err := decoder.Decode(&value); err != nil {
		return nil, err
	}
	var output bytes.Buffer
	if err := appendCanonical(&output, value); err != nil {
		return nil, err
	}
	return output.Bytes(), nil
}

func appendCanonical(output *bytes.Buffer, value any) error {
	switch typed := value.(type) {
	case nil:
		output.WriteString("null")
	case bool:
		if typed {
			output.WriteString("true")
		} else {
			output.WriteString("false")
		}
	case string:
		encoded, _ := json.Marshal(typed)
		output.Write(encoded)
	case json.Number:
		if strings.ContainsAny(typed.String(), ".eE") {
			return errors.New("canonical release JSON contains a non-integer number")
		}
		output.WriteString(typed.String())
	case []any:
		output.WriteByte('[')
		for index, item := range typed {
			if index > 0 {
				output.WriteByte(',')
			}
			if err := appendCanonical(output, item); err != nil {
				return err
			}
		}
		output.WriteByte(']')
	case map[string]any:
		keys := make([]string, 0, len(typed))
		for key := range typed {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		output.WriteByte('{')
		for index, key := range keys {
			if index > 0 {
				output.WriteByte(',')
			}
			encoded, _ := json.Marshal(key)
			output.Write(encoded)
			output.WriteByte(':')
			if err := appendCanonical(output, typed[key]); err != nil {
				return err
			}
		}
		output.WriteByte('}')
	default:
		return errors.New("canonical release JSON contains an unsupported value")
	}
	return nil
}

func verifyPackagePolicy(actual map[string]actualFile, policy platformPolicy) error {
	required := policy.required
	allowedTopLevel := map[string]struct{}{}
	for _, name := range strings.Fields(policy.topLevels) {
		allowedTopLevel[name] = struct{}{}
	}
	for name, executable := range required {
		file, ok := actual[name]
		if !ok || file.executable != executable {
			return fmt.Errorf("required release file is missing or has wrong mode: %s", name)
		}
	}
	for name := range actual {
		top := strings.SplitN(name, "/", 2)[0]
		if _, ok := allowedTopLevel[top]; !ok {
			return fmt.Errorf("release contains an unsupported top-level path: %s", top)
		}
		// Both bundles may carry the signed Windows client download payload.
		if top == "client-tools" && name != "client-tools/asterctl/windows-x86_64/asterctl.exe" {
			return fmt.Errorf("release contains an unsupported client-tools path: %s", name)
		}
		if top == "licenses" && name != "licenses/free-license.json" {
			return fmt.Errorf("release contains an unsupported licenses path: %s", name)
		}
		if name == "install.sh" || name == "restore-backup.sh" || forbiddenReleasePath(name) {
			return fmt.Errorf("release contains a forbidden source or private runtime path: %s", name)
		}
	}
	return nil
}

func verifyBundledFreeLicense(actual map[string]actualFile, task domain.ReleaseTask) error {
	if task.FreeDistributionID == "" && task.FreeLicenseSHA256 == "" {
		return nil
	}
	if task.FreeDistributionID == "" || !digestPattern.MatchString(task.FreeLicenseSHA256) {
		return errors.New("release task does not contain a complete free license identity")
	}
	license, ok := actual["licenses/free-license.json"]
	if !ok || license.executable || license.sha256 != task.FreeLicenseSHA256 {
		return errors.New("bundled free license is missing or differs from the release task")
	}
	return nil
}

func verifySBOM(contents []byte, version string, policy platformPolicy) error {
	if err := rejectDuplicateJSONKeys(contents); err != nil {
		return err
	}
	var document struct {
		Schema      string `json:"$schema"`
		BOMFormat   string `json:"bomFormat"`
		SpecVersion string `json:"specVersion"`
		Version     int    `json:"version"`
		Metadata    struct {
			Properties []struct {
				Name  string `json:"name"`
				Value string `json:"value"`
			} `json:"properties"`
			Component struct {
				Type    string `json:"type"`
				Name    string `json:"name"`
				Version string `json:"version"`
			} `json:"component"`
		} `json:"metadata"`
		Components   []json.RawMessage `json:"components"`
		Dependencies []json.RawMessage `json:"dependencies"`
	}
	decoder := json.NewDecoder(bytes.NewReader(contents))
	if err := decoder.Decode(&document); err != nil || requireJSONEOF(decoder) != nil {
		return errors.New("SBOM JSON is invalid")
	}
	if document.Schema != "https://cyclonedx.org/schema/bom-1.6.schema.json" || document.BOMFormat != "CycloneDX" ||
		document.SpecVersion != "1.6" || document.Version != 1 || document.Metadata.Component.Type != "application" ||
		document.Metadata.Component.Name != "aster-team-customer" || document.Metadata.Component.Version != version ||
		len(document.Components) == 0 || len(document.Dependencies) == 0 {
		return errors.New("SBOM identity or dependency graph is invalid")
	}
	properties := map[string]string{}
	for _, property := range document.Metadata.Properties {
		if _, exists := properties[property.Name]; exists {
			return errors.New("SBOM contains duplicate properties")
		}
		properties[property.Name] = property.Value
	}
	if properties["aster:target"] != policy.target.Platform+"-"+policy.target.Architecture || properties["aster:runtime"] != policy.sbomRuntime || properties["aster:rust-target"] != policy.rustTarget || !supportedStorageProfile(properties["aster:storage"], policy) {
		return errors.New("SBOM target or runtime does not match the package")
	}
	return nil
}

func supportedStorageProfile(storage string, policy platformPolicy) bool {
	return storage == "sqlcipher" || (storage == "sqlcipher,mariadb" && policy.target.Platform == "linux" && policy.target.Architecture == "amd64")
}

func readZipEntry(entry *zip.File, maximum int64) ([]byte, error) {
	if entry.UncompressedSize64 > uint64(maximum) {
		return nil, errors.New("ZIP entry exceeds the allowed size")
	}
	reader, err := entry.Open()
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	contents, err := io.ReadAll(io.LimitReader(reader, maximum+1))
	if err != nil || int64(len(contents)) > maximum {
		return nil, errors.New("ZIP entry is unreadable or too large")
	}
	return contents, nil
}

func rejectDuplicateJSONKeys(contents []byte) error {
	decoder := json.NewDecoder(bytes.NewReader(contents))
	decoder.UseNumber()
	if err := walkJSONValue(decoder); err != nil {
		return err
	}
	return requireJSONEOF(decoder)
}

func walkJSONValue(decoder *json.Decoder) error {
	token, err := decoder.Token()
	if err != nil {
		return errors.New("JSON is invalid")
	}
	delimiter, ok := token.(json.Delim)
	if !ok {
		return nil
	}
	switch delimiter {
	case '{':
		keys := make(map[string]struct{})
		for decoder.More() {
			keyToken, err := decoder.Token()
			key, ok := keyToken.(string)
			if err != nil || !ok {
				return errors.New("JSON object key is invalid")
			}
			if _, duplicate := keys[key]; duplicate {
				return fmt.Errorf("JSON object key is duplicated: %s", key)
			}
			keys[key] = struct{}{}
			if err := walkJSONValue(decoder); err != nil {
				return err
			}
		}
		end, err := decoder.Token()
		if err != nil || end != json.Delim('}') {
			return errors.New("JSON object is not terminated")
		}
	case '[':
		for decoder.More() {
			if err := walkJSONValue(decoder); err != nil {
				return err
			}
		}
		end, err := decoder.Token()
		if err != nil || end != json.Delim(']') {
			return errors.New("JSON array is not terminated")
		}
	default:
		return errors.New("JSON delimiter is invalid")
	}
	return nil
}

func requireJSONEOF(decoder *json.Decoder) error {
	var extra any
	if err := decoder.Decode(&extra); !errors.Is(err, io.EOF) {
		return errors.New("JSON contains trailing data")
	}
	return nil
}

func safeArchivePath(value string) bool {
	return value != "" && value == path.Clean(value) && !strings.HasPrefix(value, "/") && !strings.Contains(value, "\\") &&
		!strings.Contains(value, "//") && !strings.Contains(value, "../") && !strings.Contains(value, "/..")
}

func safeReleasePath(value string) bool {
	if value == "" || len(value) > 512 || value != path.Clean(value) || strings.HasPrefix(value, "/") || strings.Contains(value, "\\") || strings.Contains(value, "//") {
		return false
	}
	for _, character := range value {
		if character < 0x20 || character > 0x7e || character == '<' || character == '>' || character == '&' {
			return false
		}
	}
	return true
}

func forbiddenReleasePath(value string) bool {
	parts := strings.Split(value, "/")
	for _, part := range parts {
		switch strings.ToLower(part) {
		case "node_modules", "src", "source-map", ".git":
			return true
		}
	}
	return strings.HasSuffix(strings.ToLower(value), ".map") || strings.HasSuffix(strings.ToLower(value), ".cjs")
}

func exactReleaseTime(value string) bool {
	if len(value) != 24 || value[19] != '.' || !strings.HasSuffix(value, "Z") {
		return false
	}
	parsed, err := time.Parse("2006-01-02T15:04:05.000Z", value)
	return err == nil && parsed.Location() == time.UTC
}

func verifyStaticLinuxAMD64ELF(source io.ReaderAt) error {
	binary, err := elf.NewFile(source)
	if err != nil {
		return errors.New("binary is not a valid ELF")
	}
	defer binary.Close()
	if binary.Class != elf.ELFCLASS64 || binary.Data != elf.ELFDATA2LSB || binary.Machine != elf.EM_X86_64 ||
		(binary.Type != elf.ET_EXEC && binary.Type != elf.ET_DYN) {
		return errors.New("binary is not a linux/amd64 ELF executable")
	}
	for _, program := range binary.Progs {
		if program.Type == elf.PT_INTERP {
			return errors.New("binary has a dynamic interpreter")
		}
	}
	dependencies, err := binary.ImportedLibraries()
	if err != nil {
		return errors.New("binary dynamic dependencies cannot be inspected")
	}
	if len(dependencies) != 0 {
		return fmt.Errorf("binary has dynamic library dependencies: %s", strings.Join(dependencies, ", "))
	}
	return nil
}

func verifyExactDirectories(root string, files map[string]actualFile, observed map[string]struct{}) error {
	expected := map[string]struct{}{root: {}}
	for name := range files {
		current := path.Dir(name)
		for current != "." {
			expected[root+"/"+current] = struct{}{}
			current = path.Dir(current)
		}
	}
	if len(expected) != len(observed) {
		return errors.New("release archive contains missing or extra directories")
	}
	for directory := range expected {
		if _, ok := observed[directory]; !ok {
			return fmt.Errorf("release archive directory is missing: %s", directory)
		}
	}
	return nil
}

func invalid(code string, err error) error {
	return &ports.ReleaseArtifactVerificationError{Code: code, Err: err}
}
