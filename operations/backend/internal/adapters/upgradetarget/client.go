package upgradetarget

import (
	"bufio"
	"bytes"
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"io"
	"mime/multipart"
	"net/http"
	"net/http/cookiejar"
	"net/url"
	"strings"
	"sync"
	"time"

	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

type Factory struct{}
type target struct {
	environment domain.UpgradeEnvironment
	credentials domain.UpgradeCredentials
	client      *http.Client
	loginMu     sync.Mutex
}

func (Factory) Connect(env domain.UpgradeEnvironment, credentials domain.UpgradeCredentials) (ports.UpgradeTarget, error) {
	roots, err := x509.SystemCertPool()
	if err != nil {
		roots = x509.NewCertPool()
	}
	if env.CAPEM != "" && !roots.AppendCertsFromPEM([]byte(env.CAPEM)) {
		return nil, errors.New("invalid target CA")
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.Proxy = nil // Use the configured target directly, not ambient proxy credentials.
	transport.TLSClientConfig = &tls.Config{MinVersion: tls.VersionTLS12, RootCAs: roots}
	transport.MaxConnsPerHost = 8
	jar, _ := cookiejar.New(nil)
	return &target{environment: env, credentials: credentials, client: &http.Client{
		Transport: transport, Jar: jar,
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}}, nil
}

func (t *target) Close() { t.client.CloseIdleConnections() }

func (t *target) request(ctx context.Context, method, endpoint string, body io.Reader, contentType string, model bool) (*http.Response, error) {
	req, err := http.NewRequestWithContext(ctx, method, endpoint, body)
	if err != nil {
		return nil, errors.New("target_request_invalid")
	}
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	if model {
		req.Header.Set("Authorization", "Bearer "+t.credentials.APIKey)
	}
	response, err := t.client.Do(req)
	if err != nil {
		return nil, errors.New("target_transport_failed")
	}
	return response, nil
}

func (t *target) login(ctx context.Context) error {
	t.loginMu.Lock()
	defer t.loginMu.Unlock()
	encoded, _ := json.Marshal(map[string]string{"email": t.credentials.AdminEmail, "password": t.credentials.AdminPassword})
	response, err := t.request(ctx, http.MethodPost, t.environment.AdminURL+"/api/admin/auth/login", bytes.NewReader(encoded), "application/json", false)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	var result struct {
		OK                     bool `json:"ok"`
		PasswordChangeRequired bool `json:"password_change_required"`
	}
	if response.StatusCode != http.StatusOK || json.NewDecoder(io.LimitReader(response.Body, 16384)).Decode(&result) != nil || !result.OK || result.PasswordChangeRequired {
		return errors.New("target_auth_failed")
	}
	return nil
}

func (t *target) Status(ctx context.Context, requestID string) (domain.TargetMaintenance, error) {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	var value domain.TargetMaintenance
	query := url.Values{}
	if requestID != "" {
		query.Set("request_id", requestID)
	}
	for attempt := 0; attempt < 2; attempt++ {
		response, err := t.request(ctx, http.MethodGet, t.environment.AdminURL+"/api/admin/maintenance?"+query.Encode(), nil, "", false)
		if err != nil {
			return value, err
		}
		if response.StatusCode == http.StatusUnauthorized && attempt == 0 {
			response.Body.Close()
			if err = t.login(ctx); err != nil {
				return value, err
			}
			continue
		}
		err = json.NewDecoder(io.LimitReader(response.Body, 4<<20)).Decode(&value)
		response.Body.Close()
		if response.StatusCode != http.StatusOK || err != nil {
			return value, errors.New("target_status_failed")
		}
		return value, nil
	}
	return value, errors.New("target_auth_failed")
}

func (t *target) Upload(ctx context.Context, id, sha string, archive io.Reader) error {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Minute)
	defer cancel()
	reader, writer := io.Pipe()
	defer reader.Close()
	multipartWriter := multipart.NewWriter(writer)
	done := make(chan struct{})
	go func() {
		defer close(done)
		part, err := multipartWriter.CreateFormFile("package", "upgrade.tar.gz")
		if err == nil {
			_, err = io.Copy(part, archive)
		}
		if err == nil {
			err = multipartWriter.Close()
		}
		writer.CloseWithError(err)
	}()
	defer func() { reader.Close(); <-done }()
	query := url.Values{"mode": {"maintenance"}, "request_id": {id}, "expected_sha256": {sha}}
	response, err := t.request(ctx, http.MethodPost, t.environment.AdminURL+"/api/admin/maintenance/upgrade?"+query.Encode(), reader, multipartWriter.FormDataContentType(), false)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	var job domain.TargetUpgradeJob
	if response.StatusCode != http.StatusAccepted || json.NewDecoder(io.LimitReader(response.Body, 16384)).Decode(&job) != nil || job.ID != id {
		return errors.New("target_upload_unconfirmed")
	}
	return nil
}

func (t *target) Probe(ctx context.Context, kind string) domain.UpgradeProbeSample {
	start := time.Now()
	sample := domain.UpgradeProbeSample{Kind: kind, At: start.UTC()}
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	endpoint, method, contentType := t.environment.APIURL+"/healthz", http.MethodGet, ""
	var body io.Reader
	model := kind == "model" || kind == "stream"
	switch kind {
	case "admin":
		endpoint = t.environment.AdminURL + "/"
	case "member":
		endpoint = t.environment.MemberURL + "/"
	case "model", "stream":
		endpoint, method, contentType = t.environment.APIURL+"/v1/chat/completions", http.MethodPost, "application/json"
		encoded, _ := json.Marshal(map[string]any{"model": t.environment.Model, "messages": []map[string]string{{"role": "user", "content": "只返回 1"}}, "stream": kind == "stream"})
		body = bytes.NewReader(encoded)
	}
	response, err := t.request(ctx, method, endpoint, body, contentType, model)
	if err != nil {
		sample.ErrorCode = "transport_or_timeout"
	} else {
		sample.Status = response.StatusCode
		requestID := response.Header.Get("X-Request-ID")
		if safeIdentifier(requestID) {
			sample.RequestID = requestID
		}
		sample.OK = response.StatusCode == http.StatusOK
		if sample.OK {
			switch kind {
			case "admin", "member":
				data, readErr := io.ReadAll(io.LimitReader(response.Body, (2<<20)+1))
				sample.OK = readErr == nil && len(data) <= 2<<20 && strings.Contains(response.Header.Get("Content-Type"), "text/html") && bytes.Contains(bytes.ToLower(data), []byte("<html"))
			case "model":
				var result struct {
					Choices []struct {
						FinishReason string `json:"finish_reason"`
						Message      struct {
							Content string `json:"content"`
						} `json:"message"`
					} `json:"choices"`
				}
				sample.OK = json.NewDecoder(io.LimitReader(response.Body, 1<<20)).Decode(&result) == nil && len(result.Choices) > 0 && result.Choices[0].Message.Content != "" && result.Choices[0].FinishReason != ""
			case "stream":
				sample.OutputEvents, sample.OK = inspectStream(response.Body)
			default:
				_, readErr := io.Copy(io.Discard, io.LimitReader(response.Body, 16384))
				sample.OK = readErr == nil
			}
		}
		response.Body.Close()
		if !sample.OK {
			sample.ErrorCode = "http_or_protocol_failed"
		}
	}
	sample.DurationMS = time.Since(start).Milliseconds()
	return sample
}

func inspectStream(body io.Reader) (int, bool) {
	scanner := bufio.NewScanner(io.LimitReader(body, 2<<20))
	scanner.Buffer(make([]byte, 4096), 256<<10)
	events, finished, done := 0, false, false
	for scanner.Scan() {
		line := scanner.Text()
		if strings.HasPrefix(line, "event:") && strings.Contains(line, "error") {
			return events, false
		}
		if !strings.HasPrefix(line, "data:") {
			continue
		}
		data := strings.TrimSpace(strings.TrimPrefix(line, "data:"))
		if data == "[DONE]" {
			done = true
			break
		}
		var event struct {
			Error   json.RawMessage `json:"error"`
			Choices []struct {
				FinishReason *string `json:"finish_reason"`
				Delta        struct {
					Content string `json:"content"`
				} `json:"delta"`
			} `json:"choices"`
		}
		if json.Unmarshal([]byte(data), &event) != nil || (len(event.Error) > 0 && string(event.Error) != "null") {
			return events, false
		}
		for _, choice := range event.Choices {
			if choice.Delta.Content != "" {
				events++
			}
			if choice.FinishReason != nil && *choice.FinishReason != "" {
				finished = true
			}
		}
	}
	return events, scanner.Err() == nil && done && finished && events > 0
}

func safeIdentifier(value string) bool {
	if len(value) > 128 {
		return false
	}
	for _, ch := range value {
		if !(ch >= 'a' && ch <= 'z' || ch >= 'A' && ch <= 'Z' || ch >= '0' && ch <= '9' || ch == '_' || ch == '-') {
			return false
		}
	}
	return true
}
