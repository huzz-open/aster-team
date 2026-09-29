package githubactions

import (
	"bytes"
	"context"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"aster.local/team/operations/backend/internal/config"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

const apiVersion = "2026-03-10"

var (
	commitPattern       = regexp.MustCompile(`^[0-9a-f]{40}$`)
	distributionPattern = regexp.MustCompile(`^[A-Za-z0-9_.:-]{3,64}$`)
	digestPattern       = regexp.MustCompile(`^[0-9a-f]{64}$`)
	githubToken         = regexp.MustCompile(`\bgh[pousr]_[A-Za-z0-9_]{16,}\b`)
	authorizationLine   = regexp.MustCompile(`(?i)(authorization\s*:\s*)([^\r\n]+)`)
	secretAssignment    = regexp.MustCompile(`(?i)(authorization|cookie|token|password|secret|private[_ -]?key)(\s*[:=]\s*)([^\s]+)`)
)

type Client struct {
	config             config.GitHubApp
	httpClient         *http.Client
	artifactHTTPClient *http.Client
	privateKey         *rsa.PrivateKey
	now                func() time.Time
	permissions        map[string]string

	tokenMu      sync.Mutex
	token        string
	tokenExpires time.Time
}

func New(cfg config.GitHubApp) (*Client, error) {
	if !cfg.Enabled {
		return nil, errors.New("GitHub release orchestration is disabled")
	}
	keyBytes, err := base64.StdEncoding.DecodeString(cfg.PrivateKeyPEMBase64)
	if err != nil {
		keyBytes, err = base64.RawStdEncoding.DecodeString(cfg.PrivateKeyPEMBase64)
	}
	if err != nil {
		return nil, errors.New("decode GitHub App private key")
	}
	privateKey, err := parsePrivateKey(keyBytes)
	if err != nil {
		return nil, err
	}
	return &Client{config: cfg, privateKey: privateKey, now: time.Now,
		httpClient: &http.Client{Timeout: cfg.RequestTimeout}, artifactHTTPClient: &http.Client{Timeout: cfg.ArtifactDownloadTimeout},
		permissions: map[string]string{"actions": "write", "contents": "read", "environments": "read"}}, nil
}

func (client *Client) Capabilities() domain.ReleaseCapabilities {
	return domain.ReleaseCapabilities{Configured: true, Repository: client.config.Repository, WorkflowFile: client.config.WorkflowFile,
		Environment: client.config.Environment, DefaultSourceRef: client.config.DefaultSourceRef, Targets: domain.ReleaseTargets()}
}

func (client *Client) Preflight(ctx context.Context, version, sourceRef string) (ports.ReleasePreflight, error) {
	version = strings.TrimSpace(version)
	sourceRef = strings.TrimSpace(sourceRef)
	if !domain.ValidSemanticVersion(version) || (sourceRef != client.config.DefaultSourceRef && !commitPattern.MatchString(sourceRef)) {
		return ports.ReleasePreflight{}, errors.New("release version or source ref is invalid")
	}
	var commit struct {
		SHA string `json:"sha"`
	}
	if err := client.getJSON(ctx, client.repoPath("commits/"+url.PathEscape(sourceRef)), &commit); err != nil {
		return ports.ReleasePreflight{}, fmt.Errorf("resolve release source: %w", err)
	}
	if !commitPattern.MatchString(commit.SHA) {
		return ports.ReleasePreflight{}, errors.New("GitHub returned an invalid commit SHA")
	}
	if sourceRef != client.config.DefaultSourceRef {
		var comparison struct {
			Status string `json:"status"`
		}
		comparisonPath := client.repoPath("compare/" + url.PathEscape(commit.SHA+"..."+client.config.DefaultSourceRef))
		if err := client.getJSON(ctx, comparisonPath, &comparison); err != nil {
			return ports.ReleasePreflight{}, fmt.Errorf("compare release source with main: %w", err)
		}
		if comparison.Status != "ahead" && comparison.Status != "identical" {
			return ports.ReleasePreflight{}, errors.New("release source commit is not an ancestor of main")
		}
	}
	var packageFile struct {
		Encoding string `json:"encoding"`
		Content  string `json:"content"`
	}
	packagePath := client.repoPath("contents/package.json") + "?ref=" + url.QueryEscape(commit.SHA)
	if err := client.getJSON(ctx, packagePath, &packageFile); err != nil {
		return ports.ReleasePreflight{}, fmt.Errorf("read package version: %w", err)
	}
	if packageFile.Encoding != "base64" {
		return ports.ReleasePreflight{}, errors.New("package.json content encoding is not base64")
	}
	packageBytes, err := base64.StdEncoding.DecodeString(strings.ReplaceAll(packageFile.Content, "\n", ""))
	if err != nil || len(packageBytes) > 1024*1024 {
		return ports.ReleasePreflight{}, errors.New("package.json content is invalid")
	}
	var packageMetadata struct {
		Version string `json:"version"`
	}
	if err := json.Unmarshal(packageBytes, &packageMetadata); err != nil || packageMetadata.Version != version {
		return ports.ReleasePreflight{}, errors.New("requested release version does not match package.json at the source commit")
	}
	var workflow struct {
		State string `json:"state"`
	}
	if err := client.getJSON(ctx, client.repoPath("actions/workflows/"+url.PathEscape(client.config.WorkflowFile)), &workflow); err != nil {
		return ports.ReleasePreflight{}, fmt.Errorf("read release workflow: %w", err)
	}
	if workflow.State != "active" {
		return ports.ReleasePreflight{}, errors.New("release workflow is not active")
	}
	for _, name := range []string{"ASTER_RELEASE_SIGNING_SEED_BASE64"} {
		path := client.repoPath("environments/" + url.PathEscape(client.config.Environment) + "/secrets/" + url.PathEscape(name))
		if err := client.getJSON(ctx, path, &struct{}{}); err != nil {
			return ports.ReleasePreflight{}, fmt.Errorf("required GitHub environment secret %s is unavailable: %w", name, err)
		}
	}
	for _, name := range []string{"ASTER_LICENSE_TRUSTED_KEYS_JSON", "ASTER_RELEASE_TRUSTED_KEYS_JSON", "ASTER_RELEASE_SIGNING_KEY_ID", "ASTER_CUSTOMER_FREE_LICENSE_BASE64", "ASTER_CUSTOMER_FREE_LICENSE_SHA256"} {
		path := client.repoPath("environments/" + url.PathEscape(client.config.Environment) + "/variables/" + url.PathEscape(name))
		if err := client.getJSON(ctx, path, &struct{}{}); err != nil {
			return ports.ReleasePreflight{}, fmt.Errorf("required GitHub environment variable %s is unavailable: %w", name, err)
		}
	}
	return ports.ReleasePreflight{CommitSHA: commit.SHA}, nil
}

func (client *Client) Dispatch(ctx context.Context, input ports.ReleaseDispatchInput) (ports.ReleaseDispatch, error) {
	if input.SourceRef != client.config.DefaultSourceRef && !commitPattern.MatchString(input.SourceRef) {
		return ports.ReleaseDispatch{}, errors.New("release source ref is invalid")
	}
	if !distributionPattern.MatchString(input.TaskID) || !domain.ValidSemanticVersion(input.Version) || !commitPattern.MatchString(input.SourceCommitSHA) ||
		!distributionPattern.MatchString(input.FreeDistributionID) || !digestPattern.MatchString(input.FreeLicenseSHA256) ||
		len(input.FreeLicenseDocument) == 0 || len(input.FreeLicenseDocument) > 32<<10 {
		return ports.ReleaseDispatch{}, errors.New("release dispatch input is invalid")
	}
	digest := sha256.Sum256(input.FreeLicenseDocument)
	if fmt.Sprintf("%x", digest) != input.FreeLicenseSHA256 {
		return ports.ReleaseDispatch{}, errors.New("free license digest does not match the release task")
	}
	body := map[string]any{"ref": client.config.DefaultSourceRef, "inputs": map[string]string{
		"version": input.Version, "release_task_id": input.TaskID, "source_commit_sha": input.SourceCommitSHA,
		"free_distribution_id": input.FreeDistributionID, "free_license_sha256": input.FreeLicenseSHA256,
		"free_license_base64": base64.StdEncoding.EncodeToString(input.FreeLicenseDocument),
	}}
	var response struct {
		WorkflowRunID int64  `json:"workflow_run_id"`
		RunURL        string `json:"run_url"`
		HTMLURL       string `json:"html_url"`
	}
	path := client.repoPath("actions/workflows/" + url.PathEscape(client.config.WorkflowFile) + "/dispatches")
	if err := client.doJSON(ctx, http.MethodPost, path, body, &response); err != nil {
		return ports.ReleaseDispatch{}, err
	}
	if response.WorkflowRunID <= 0 || response.HTMLURL == "" {
		return ports.ReleaseDispatch{}, errors.New("GitHub dispatch response did not contain a workflow run")
	}
	return ports.ReleaseDispatch{RunID: response.WorkflowRunID, RunURL: response.HTMLURL, Status: "queued", CreatedAt: client.now().UTC()}, nil
}

func (client *Client) Snapshot(ctx context.Context, task domain.ReleaseTask, runID int64) (ports.ReleaseSnapshot, error) {
	var run githubRun
	if err := client.getJSON(ctx, client.repoPath("actions/runs/"+strconv.FormatInt(runID, 10)), &run); err != nil {
		return ports.ReleaseSnapshot{}, err
	}
	if run.ID != runID || !strings.Contains(run.DisplayTitle, task.ID) || !strings.Contains(run.DisplayTitle, task.SourceCommitSHA) {
		return ports.ReleaseSnapshot{}, errors.New("GitHub workflow run is not bound to the release task")
	}
	domainRun := domain.ReleaseRun{ID: releaseRunID(run.ID), ReleaseTaskID: task.ID, Attempt: run.RunAttempt, GitHubRunID: &run.ID,
		GitHubRunNumber: &run.RunNumber, WorkflowName: run.Name, HeadBranch: task.SourceRef, HeadSHA: task.SourceCommitSHA,
		Status: run.Status, Conclusion: optionalString(run.Conclusion), HTMLURL: run.HTMLURL, StartedAt: optionalTime(run.RunStartedAt),
		CompletedAt: completedAt(run.Status, run.UpdatedAt), SyncedAt: timePointer(client.now().UTC()), CreatedAt: run.CreatedAt, Jobs: []domain.ReleaseJob{}}

	var jobsResponse struct {
		Jobs []githubJob `json:"jobs"`
	}
	jobsPath := client.repoPath("actions/runs/" + strconv.FormatInt(runID, 10) + "/jobs?filter=latest&per_page=100")
	if err := client.getJSON(ctx, jobsPath, &jobsResponse); err != nil {
		return ports.ReleaseSnapshot{}, err
	}
	for jobIndex, job := range jobsResponse.Jobs {
		failureSummary := ""
		if job.Conclusion == "failure" || job.Conclusion == "timed_out" || job.Conclusion == "startup_failure" {
			if logBytes, err := client.getBytes(ctx, client.repoPath("actions/jobs/"+strconv.FormatInt(job.ID, 10)+"/logs"), 2*1024*1024); err == nil {
				failureSummary = sanitizeFailureSummary(logBytes)
			}
		}
		domainJob := domain.ReleaseJob{ID: releaseJobID(job.ID), ReleaseRunID: domainRun.ID, GitHubJobID: job.ID,
			SequenceNo: jobIndex, Name: job.Name, RunnerName: job.RunnerName, Status: job.Status,
			Conclusion: optionalString(job.Conclusion), HTMLURL: job.HTMLURL, StartedAt: optionalTime(job.StartedAt),
			CompletedAt: optionalTime(job.CompletedAt), Steps: []domain.ReleaseStep{}}
		for stepIndex, step := range job.Steps {
			summary := ""
			if step.Conclusion == "failure" || step.Conclusion == "timed_out" || step.Conclusion == "startup_failure" {
				summary = failureSummary
			}
			domainJob.Steps = append(domainJob.Steps, domain.ReleaseStep{ID: releaseStepID(job.ID, step.Number), ReleaseRunJobID: domainJob.ID,
				GitHubStepNumber: step.Number, SequenceNo: stepIndex, Name: step.Name, Status: step.Status,
				Conclusion: optionalString(step.Conclusion), FailureSummary: summary, LogRef: job.HTMLURL,
				StartedAt: optionalTime(step.StartedAt), CompletedAt: optionalTime(step.CompletedAt)})
		}
		domainRun.Jobs = append(domainRun.Jobs, domainJob)
	}

	var artifactResponse struct {
		Artifacts []githubArtifact `json:"artifacts"`
	}
	artifactPath := client.repoPath("actions/runs/" + strconv.FormatInt(runID, 10) + "/artifacts?per_page=100")
	if err := client.getJSON(ctx, artifactPath, &artifactResponse); err != nil {
		return ports.ReleaseSnapshot{}, err
	}
	artifacts := make([]domain.ReleaseTaskArtifact, 0, len(artifactResponse.Artifacts))
	for _, artifact := range artifactResponse.Artifacts {
		verificationStatus := "pending"
		if artifact.Expired {
			verificationStatus = "unavailable"
		}
		var target domain.ReleaseTarget
		fileName := artifact.Name + ".zip"
		for _, supported := range domain.ReleaseTargets() {
			if artifact.Name == supported.ArtifactName(task.Version) {
				target = supported
				fileName = target.FileName(task.Version)
				break
			}
		}
		artifacts = append(artifacts, domain.ReleaseTaskArtifact{Platform: target.Platform, Architecture: target.Architecture, ID: releaseTaskArtifactID(artifact.ID), ReleaseTaskID: task.ID,
			ReleaseRunID: domainRun.ID, GitHubArtifactID: artifact.ID, Name: artifact.Name, FileName: fileName,
			SizeBytes: artifact.SizeBytes, ExpiresAt: optionalTime(artifact.ExpiresAt), DownloadRef: artifact.ArchiveDownloadURL,
			GitHubDigestSHA256: artifactDigest(artifact.Digest), SignatureKeyID: "", VerificationStatus: verificationStatus, CreatedAt: artifact.CreatedAt})
	}
	return ports.ReleaseSnapshot{Run: domainRun, Artifacts: artifacts}, nil
}

func (client *Client) DownloadArtifact(ctx context.Context, artifactID int64) (io.ReadCloser, error) {
	if artifactID <= 0 {
		return nil, errors.New("GitHub artifact ID is invalid")
	}
	response, err := client.doInstallationWithClient(ctx, client.artifactHTTPClient, http.MethodGet,
		client.repoPath("actions/artifacts/"+strconv.FormatInt(artifactID, 10)+"/zip"), nil)
	if err != nil {
		return nil, err
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		defer response.Body.Close()
		if response.StatusCode == http.StatusNotFound || response.StatusCode == http.StatusGone {
			return nil, &ports.ReleaseArtifactVerificationError{Code: "RELEASE_ARTIFACT_UNAVAILABLE", Err: responseError(response)}
		}
		return nil, responseError(response)
	}
	return response.Body, nil
}

type githubRun struct {
	ID           int64     `json:"id"`
	RunNumber    int64     `json:"run_number"`
	RunAttempt   int       `json:"run_attempt"`
	Name         string    `json:"name"`
	DisplayTitle string    `json:"display_title"`
	HeadBranch   string    `json:"head_branch"`
	HeadSHA      string    `json:"head_sha"`
	Status       string    `json:"status"`
	Conclusion   string    `json:"conclusion"`
	HTMLURL      string    `json:"html_url"`
	RunStartedAt time.Time `json:"run_started_at"`
	CreatedAt    time.Time `json:"created_at"`
	UpdatedAt    time.Time `json:"updated_at"`
}

type githubJob struct {
	ID          int64        `json:"id"`
	Name        string       `json:"name"`
	RunnerName  string       `json:"runner_name"`
	Status      string       `json:"status"`
	Conclusion  string       `json:"conclusion"`
	HTMLURL     string       `json:"html_url"`
	StartedAt   time.Time    `json:"started_at"`
	CompletedAt time.Time    `json:"completed_at"`
	Steps       []githubStep `json:"steps"`
}

type githubStep struct {
	Name        string    `json:"name"`
	Status      string    `json:"status"`
	Conclusion  string    `json:"conclusion"`
	Number      int       `json:"number"`
	StartedAt   time.Time `json:"started_at"`
	CompletedAt time.Time `json:"completed_at"`
}

type githubArtifact struct {
	ID                 int64     `json:"id"`
	Name               string    `json:"name"`
	SizeBytes          int64     `json:"size_in_bytes"`
	Expired            bool      `json:"expired"`
	CreatedAt          time.Time `json:"created_at"`
	ExpiresAt          time.Time `json:"expires_at"`
	Digest             string    `json:"digest"`
	ArchiveDownloadURL string    `json:"archive_download_url"`
}

func (client *Client) repoPath(suffix string) string {
	return "/repos/" + client.config.Repository + "/" + suffix
}

func (client *Client) getJSON(ctx context.Context, path string, destination any) error {
	return client.doJSON(ctx, http.MethodGet, path, nil, destination)
}

func (client *Client) doJSON(ctx context.Context, method, path string, body any, destination any) error {
	var encoded []byte
	var err error
	if body != nil {
		encoded, err = json.Marshal(body)
		if err != nil {
			return err
		}
	}
	response, err := client.doInstallation(ctx, method, path, encoded)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return responseError(response)
	}
	if destination == nil || response.StatusCode == http.StatusNoContent {
		return nil
	}
	decoder := json.NewDecoder(io.LimitReader(response.Body, 4*1024*1024))
	if err := decoder.Decode(destination); err != nil {
		return errors.New("decode GitHub API response")
	}
	return nil
}

func (client *Client) getBytes(ctx context.Context, path string, maximum int64) ([]byte, error) {
	response, err := client.doInstallation(ctx, http.MethodGet, path, nil)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, responseError(response)
	}
	contents, err := io.ReadAll(io.LimitReader(response.Body, maximum+1))
	if err != nil || int64(len(contents)) > maximum {
		return nil, errors.New("GitHub response exceeds the allowed size")
	}
	return contents, nil
}

func (client *Client) doInstallation(ctx context.Context, method, path string, body []byte) (*http.Response, error) {
	return client.doInstallationWithClient(ctx, client.httpClient, method, path, body)
}

func (client *Client) doInstallationWithClient(ctx context.Context, httpClient *http.Client, method, path string, body []byte) (*http.Response, error) {
	for attempt := 0; attempt < 2; attempt++ {
		token, err := client.installationToken(ctx)
		if err != nil {
			return nil, err
		}
		request, err := client.request(ctx, method, path, body, token)
		if err != nil {
			return nil, err
		}
		response, err := httpClient.Do(request)
		if err != nil {
			return nil, errors.New("GitHub API request failed")
		}
		if response.StatusCode != http.StatusUnauthorized || attempt == 1 {
			return response, nil
		}
		response.Body.Close()
		client.invalidateToken()
	}
	return nil, errors.New("GitHub authentication failed")
}

func (client *Client) installationToken(ctx context.Context) (string, error) {
	client.tokenMu.Lock()
	defer client.tokenMu.Unlock()
	now := client.now().UTC()
	if client.token != "" && client.tokenExpires.After(now.Add(5*time.Minute)) {
		return client.token, nil
	}
	jwt, err := client.appJWT(now)
	if err != nil {
		return "", err
	}
	repository := strings.Split(client.config.Repository, "/")[1]
	body, err := json.Marshal(map[string]any{"repositories": []string{repository}, "permissions": client.permissions})
	if err != nil {
		return "", err
	}
	path := "/app/installations/" + strconv.FormatInt(client.config.InstallationID, 10) + "/access_tokens"
	request, err := client.request(ctx, http.MethodPost, path, body, jwt)
	if err != nil {
		return "", err
	}
	response, err := client.httpClient.Do(request)
	if err != nil {
		return "", errors.New("request GitHub App installation token failed")
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return "", responseError(response)
	}
	var tokenResponse struct {
		Token     string    `json:"token"`
		ExpiresAt time.Time `json:"expires_at"`
	}
	if err := json.NewDecoder(io.LimitReader(response.Body, 1024*1024)).Decode(&tokenResponse); err != nil || tokenResponse.Token == "" || !tokenResponse.ExpiresAt.After(now) {
		return "", errors.New("GitHub App installation token response is invalid")
	}
	client.token, client.tokenExpires = tokenResponse.Token, tokenResponse.ExpiresAt
	return client.token, nil
}

func (client *Client) invalidateToken() {
	client.tokenMu.Lock()
	client.token, client.tokenExpires = "", time.Time{}
	client.tokenMu.Unlock()
}

func (client *Client) request(ctx context.Context, method, path string, body []byte, token string) (*http.Request, error) {
	base := strings.TrimRight(client.config.APIBaseURL, "/")
	request, err := http.NewRequestWithContext(ctx, method, base+path, bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	request.Header.Set("Accept", "application/vnd.github+json")
	request.Header.Set("Authorization", "Bearer "+token)
	request.Header.Set("X-GitHub-Api-Version", apiVersion)
	request.Header.Set("User-Agent", "aster-operations-release-center")
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	return request, nil
}

func (client *Client) appJWT(now time.Time) (string, error) {
	header := base64.RawURLEncoding.EncodeToString([]byte(`{"alg":"RS256","typ":"JWT"}`))
	payloadBytes, err := json.Marshal(map[string]any{"iat": now.Add(-60 * time.Second).Unix(), "exp": now.Add(9 * time.Minute).Unix(), "iss": client.config.AppID})
	if err != nil {
		return "", err
	}
	payload := base64.RawURLEncoding.EncodeToString(payloadBytes)
	unsigned := header + "." + payload
	digest := sha256.Sum256([]byte(unsigned))
	signature, err := rsa.SignPKCS1v15(rand.Reader, client.privateKey, crypto.SHA256, digest[:])
	if err != nil {
		return "", errors.New("sign GitHub App JWT")
	}
	return unsigned + "." + base64.RawURLEncoding.EncodeToString(signature), nil
}

func parsePrivateKey(contents []byte) (*rsa.PrivateKey, error) {
	block, _ := pem.Decode(contents)
	if block == nil {
		return nil, errors.New("GitHub App private key is not PEM")
	}
	if key, err := x509.ParsePKCS1PrivateKey(block.Bytes); err == nil {
		return key, nil
	}
	parsed, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, errors.New("GitHub App private key is invalid")
	}
	key, ok := parsed.(*rsa.PrivateKey)
	if !ok {
		return nil, errors.New("GitHub App private key must be RSA")
	}
	return key, nil
}

func responseError(response *http.Response) error {
	contents, _ := io.ReadAll(io.LimitReader(response.Body, 32*1024))
	message := sanitizeFailureSummary(contents)
	if message == "" {
		message = http.StatusText(response.StatusCode)
	}
	return fmt.Errorf("GitHub API returned %d: %s", response.StatusCode, message)
}

func sanitizeFailureSummary(contents []byte) string {
	value := strings.ReplaceAll(string(contents), "\x00", "")
	value = githubToken.ReplaceAllString(value, "[REDACTED]")
	value = authorizationLine.ReplaceAllString(value, "$1[REDACTED]")
	value = secretAssignment.ReplaceAllString(value, "$1$2[REDACTED]")
	value = strings.ReplaceAll(value, "-----BEGIN PRIVATE KEY-----", "[REDACTED PRIVATE KEY]")
	value = strings.ReplaceAll(value, "-----BEGIN RSA PRIVATE KEY-----", "[REDACTED PRIVATE KEY]")
	lines := strings.Split(value, "\n")
	if len(lines) > 80 {
		lines = lines[len(lines)-80:]
	}
	value = strings.TrimSpace(strings.Join(lines, "\n"))
	if len(value) > 4000 {
		value = value[len(value)-4000:]
	}
	return value
}

func releaseRunID(id int64) string { return "release_run_" + strconv.FormatInt(id, 10) }
func releaseJobID(id int64) string { return "release_job_" + strconv.FormatInt(id, 10) }
func releaseStepID(jobID int64, number int) string {
	return "release_step_" + strconv.FormatInt(jobID, 10) + "_" + strconv.Itoa(number)
}
func releaseTaskArtifactID(id int64) string {
	return "release_task_artifact_" + strconv.FormatInt(id, 10)
}
func optionalString(value string) *string {
	if value == "" {
		return nil
	}
	return &value
}
func optionalTime(value time.Time) *time.Time {
	if value.IsZero() {
		return nil
	}
	value = value.UTC()
	return &value
}
func timePointer(value time.Time) *time.Time { value = value.UTC(); return &value }
func completedAt(status string, value time.Time) *time.Time {
	if status != "completed" {
		return nil
	}
	return optionalTime(value)
}
func artifactDigest(value string) *string {
	digest := strings.TrimPrefix(strings.ToLower(strings.TrimSpace(value)), "sha256:")
	if len(digest) != 64 {
		return nil
	}
	for _, character := range digest {
		if (character < '0' || character > '9') && (character < 'a' || character > 'f') {
			return nil
		}
	}
	return &digest
}
