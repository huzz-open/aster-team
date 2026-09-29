package githubactions

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"

	"aster.local/team/operations/backend/internal/config"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

var sha256DigestPattern = regexp.MustCompile(`^[0-9a-f]{64}$`)

type Publisher struct {
	client           *Client
	store            ports.ArtifactStore
	uploadBaseURL    string
	maxArtifactBytes int64
}

type githubRelease struct {
	ID      int64  `json:"id"`
	TagName string `json:"tag_name"`
	Draft   bool   `json:"draft"`
	HTMLURL string `json:"html_url"`
}

type githubReleaseAsset struct {
	ID     int64  `json:"id"`
	Name   string `json:"name"`
	Size   int64  `json:"size"`
	Digest string `json:"digest"`
}

func NewPublisher(cfg config.GitHubPublisher, store ports.ArtifactStore, maxArtifactBytes int64) (*Publisher, error) {
	if !cfg.Enabled || store == nil || maxArtifactBytes < 1 {
		return nil, errors.New("GitHub release publishing is disabled or incomplete")
	}
	client, err := New(config.GitHubApp{Enabled: true, APIBaseURL: cfg.APIBaseURL, AppID: cfg.AppID,
		InstallationID: cfg.InstallationID, PrivateKeyPEMBase64: cfg.PrivateKeyPEMBase64,
		Repository: cfg.Repository, RequestTimeout: cfg.RequestTimeout, ArtifactDownloadTimeout: cfg.RequestTimeout})
	if err != nil {
		return nil, err
	}
	client.permissions = map[string]string{"contents": "write"}
	return &Publisher{client: client, store: store, uploadBaseURL: strings.TrimRight(cfg.UploadBaseURL, "/"), maxArtifactBytes: maxArtifactBytes}, nil
}

func (*Publisher) Configured() bool { return true }

func (publisher *Publisher) Publish(ctx context.Context, release domain.ReleaseArtifact) (ports.ReleasePublishResult, error) {
	if release.SourceCommitSHA == nil || !commitPattern.MatchString(*release.SourceCommitSHA) || !domain.ValidSemanticVersion(release.Version) ||
		release.Platform != "linux" || release.Architecture != "amd64" || !sha256DigestPattern.MatchString(release.SHA256) ||
		release.SizeBytes < 1 || release.SizeBytes > publisher.maxArtifactBytes {
		return ports.ReleasePublishResult{}, errors.New("verified release provenance is incomplete")
	}
	tagName := "v" + release.Version
	if err := publisher.ensureTag(ctx, tagName, *release.SourceCommitSHA); err != nil {
		return ports.ReleasePublishResult{}, err
	}
	releaseRecord, err := publisher.ensureDraftRelease(ctx, tagName, *release.SourceCommitSHA)
	if err != nil {
		return ports.ReleasePublishResult{}, err
	}
	archiveName := fmt.Sprintf("aster-team-%s-linux-amd64.tar.gz", release.Version)
	checksumName := archiveName + ".sha256"
	checksum := []byte(release.SHA256 + "  " + archiveName + "\n")
	checksumDigest := sha256.Sum256(checksum)
	expected := map[string]assetExpectation{
		archiveName:  {size: release.SizeBytes, sha256: release.SHA256, contentType: "application/gzip"},
		checksumName: {size: int64(len(checksum)), sha256: hex.EncodeToString(checksumDigest[:]), contentType: "text/plain; charset=utf-8", contents: checksum},
	}
	assets, err := publisher.listAssets(ctx, releaseRecord.ID)
	if err != nil {
		return ports.ReleasePublishResult{}, err
	}
	if err := validateExistingAssets(assets, expected); err != nil {
		return ports.ReleasePublishResult{}, err
	}
	existing := make(map[string]struct{}, len(assets))
	for _, asset := range assets {
		existing[asset.Name] = struct{}{}
	}
	for _, name := range []string{archiveName, checksumName} {
		if _, ok := existing[name]; ok {
			continue
		}
		expectation := expected[name]
		var source ports.ReadSeekCloser
		if expectation.contents != nil {
			source = &memoryReadSeekCloser{Reader: bytes.NewReader(expectation.contents)}
		} else {
			opened, size, err := publisher.store.OpenObject(ctx, release.ObjectKey, publisher.maxArtifactBytes)
			if err != nil {
				return ports.ReleasePublishResult{}, fmt.Errorf("open verified release object: %w", err)
			}
			if size != expectation.size {
				opened.Close()
				return ports.ReleasePublishResult{}, errors.New("verified release object size changed")
			}
			source = opened
		}
		asset, err := publisher.uploadAsset(ctx, releaseRecord.ID, name, expectation, source)
		source.Close()
		if err != nil {
			return ports.ReleasePublishResult{}, err
		}
		if err := validateAsset(asset, name, expectation); err != nil {
			return ports.ReleasePublishResult{}, err
		}
	}
	assets, err = publisher.listAssets(ctx, releaseRecord.ID)
	if err != nil {
		return ports.ReleasePublishResult{}, err
	}
	if err := validateExistingAssets(assets, expected); err != nil || len(assets) != len(expected) {
		if err == nil {
			err = errors.New("GitHub draft release has unexpected assets")
		}
		return ports.ReleasePublishResult{}, err
	}
	if releaseRecord.Draft {
		var published githubRelease
		if err := publisher.client.doJSON(ctx, http.MethodPatch, publisher.client.repoPath("releases/"+strconv.FormatInt(releaseRecord.ID, 10)),
			map[string]any{"draft": false, "make_latest": "true"}, &published); err != nil {
			return ports.ReleasePublishResult{}, fmt.Errorf("publish GitHub draft release: %w", err)
		}
		if published.ID != releaseRecord.ID || published.Draft || published.TagName != tagName || published.HTMLURL == "" {
			return ports.ReleasePublishResult{}, errors.New("GitHub returned an invalid published release")
		}
		releaseRecord = published
	}
	return ports.ReleasePublishResult{GitHubReleaseID: releaseRecord.ID, HTMLURL: releaseRecord.HTMLURL, TagName: tagName}, nil
}

func (publisher *Publisher) ensureTag(ctx context.Context, tagName, commitSHA string) error {
	var reference struct {
		Ref    string `json:"ref"`
		Object struct {
			Type string `json:"type"`
			SHA  string `json:"sha"`
		} `json:"object"`
	}
	found, err := publisher.optionalJSON(ctx, publisher.client.repoPath("git/ref/tags/"+url.PathEscape(tagName)), &reference)
	if err != nil {
		return fmt.Errorf("read release tag: %w", err)
	}
	if !found {
		if err := publisher.client.doJSON(ctx, http.MethodPost, publisher.client.repoPath("git/refs"),
			map[string]string{"ref": "refs/tags/" + tagName, "sha": commitSHA}, &reference); err != nil {
			return fmt.Errorf("create protected release tag: %w", err)
		}
	}
	if reference.Ref != "refs/tags/"+tagName || reference.Object.Type != "commit" || reference.Object.SHA != commitSHA {
		return errors.New("release tag exists but does not point to the approved commit")
	}
	return nil
}

func (publisher *Publisher) ensureDraftRelease(ctx context.Context, tagName, commitSHA string) (githubRelease, error) {
	var release githubRelease
	found, err := publisher.optionalJSON(ctx, publisher.client.repoPath("releases/tags/"+url.PathEscape(tagName)), &release)
	if err != nil {
		return githubRelease{}, fmt.Errorf("read GitHub release: %w", err)
	}
	if !found {
		err = publisher.client.doJSON(ctx, http.MethodPost, publisher.client.repoPath("releases"), map[string]any{
			"tag_name": tagName, "target_commitish": commitSHA, "name": "Aster Team " + strings.TrimPrefix(tagName, "v"),
			"draft": true, "prerelease": false, "generate_release_notes": true,
		}, &release)
		if err != nil {
			return githubRelease{}, fmt.Errorf("create GitHub draft release: %w", err)
		}
	}
	if release.ID <= 0 || release.TagName != tagName || release.HTMLURL == "" {
		return githubRelease{}, errors.New("GitHub release identity is invalid")
	}
	return release, nil
}

func (publisher *Publisher) optionalJSON(ctx context.Context, path string, destination any) (bool, error) {
	response, err := publisher.client.doInstallation(ctx, http.MethodGet, path, nil)
	if err != nil {
		return false, err
	}
	defer response.Body.Close()
	if response.StatusCode == http.StatusNotFound {
		return false, nil
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return false, responseError(response)
	}
	if err := json.NewDecoder(io.LimitReader(response.Body, 2<<20)).Decode(destination); err != nil {
		return false, errors.New("decode GitHub API response")
	}
	return true, nil
}

func (publisher *Publisher) listAssets(ctx context.Context, releaseID int64) ([]githubReleaseAsset, error) {
	var assets []githubReleaseAsset
	if err := publisher.client.getJSON(ctx, publisher.client.repoPath("releases/"+strconv.FormatInt(releaseID, 10)+"/assets?per_page=100"), &assets); err != nil {
		return nil, fmt.Errorf("list GitHub release assets: %w", err)
	}
	return assets, nil
}

type assetExpectation struct {
	size        int64
	sha256      string
	contentType string
	contents    []byte
}

func validateExistingAssets(assets []githubReleaseAsset, expected map[string]assetExpectation) error {
	seen := make(map[string]struct{}, len(assets))
	for _, asset := range assets {
		expectation, ok := expected[asset.Name]
		if !ok {
			return fmt.Errorf("GitHub release contains an unexpected asset: %s", asset.Name)
		}
		if _, duplicate := seen[asset.Name]; duplicate {
			return fmt.Errorf("GitHub release contains a duplicated asset: %s", asset.Name)
		}
		seen[asset.Name] = struct{}{}
		if err := validateAsset(asset, asset.Name, expectation); err != nil {
			return err
		}
	}
	return nil
}

func validateAsset(asset githubReleaseAsset, name string, expected assetExpectation) error {
	if asset.ID <= 0 || asset.Name != name || asset.Size != expected.size || asset.Digest != "sha256:"+expected.sha256 {
		return fmt.Errorf("GitHub release asset does not match the approved bytes: %s", name)
	}
	return nil
}

func (publisher *Publisher) uploadAsset(ctx context.Context, releaseID int64, name string, expected assetExpectation, source ports.ReadSeekCloser) (githubReleaseAsset, error) {
	endpoint := publisher.uploadBaseURL + "/repos/" + publisher.client.config.Repository + "/releases/" + strconv.FormatInt(releaseID, 10) + "/assets?name=" + url.QueryEscape(name)
	for attempt := 0; attempt < 2; attempt++ {
		if _, err := source.Seek(0, io.SeekStart); err != nil {
			return githubReleaseAsset{}, err
		}
		token, err := publisher.client.installationToken(ctx)
		if err != nil {
			return githubReleaseAsset{}, err
		}
		request, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, io.LimitReader(source, expected.size))
		if err != nil {
			return githubReleaseAsset{}, err
		}
		request.ContentLength = expected.size
		request.Header.Set("Accept", "application/vnd.github+json")
		request.Header.Set("Authorization", "Bearer "+token)
		request.Header.Set("X-GitHub-Api-Version", apiVersion)
		request.Header.Set("User-Agent", "aster-operations-release-center")
		request.Header.Set("Content-Type", expected.contentType)
		response, err := publisher.client.httpClient.Do(request)
		if err != nil {
			return githubReleaseAsset{}, errors.New("GitHub asset upload failed")
		}
		if response.StatusCode == http.StatusUnauthorized && attempt == 0 {
			response.Body.Close()
			publisher.client.invalidateToken()
			continue
		}
		defer response.Body.Close()
		if response.StatusCode < 200 || response.StatusCode >= 300 {
			return githubReleaseAsset{}, responseError(response)
		}
		var asset githubReleaseAsset
		if err := json.NewDecoder(io.LimitReader(response.Body, 2<<20)).Decode(&asset); err != nil {
			return githubReleaseAsset{}, errors.New("decode GitHub asset upload response")
		}
		return asset, nil
	}
	return githubReleaseAsset{}, errors.New("GitHub asset upload authentication failed")
}

type memoryReadSeekCloser struct{ *bytes.Reader }

func (*memoryReadSeekCloser) Close() error { return nil }
