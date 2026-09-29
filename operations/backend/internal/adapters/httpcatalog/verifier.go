// Package httpcatalog reconciles a controlled website target. It has no deploy,
// signing, DNS or remote mutation capability.
package httpcatalog

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"path"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/strictjson"
)

const manifestLimit = 256 << 10
const fileLimit = 10 << 20
const runtimeLimit = 32 << 20

type Verifier struct {
	environment string
	origin      string
	client      *http.Client
	now         func() time.Time
	host        string
	lookup      func(context.Context, string) ([]net.IPAddr, error)
	dial        func(context.Context, string, string) (net.Conn, error)
}

func New(environment, origin string) (*Verifier, error) {
	if err := commercial.ValidatePublicationOrigin(environment, origin); err != nil {
		return nil, err
	}
	u, _ := url.Parse(origin)
	dialer := &net.Dialer{Timeout: 10 * time.Second, KeepAlive: 30 * time.Second}
	v := &Verifier{environment: environment, origin: origin, now: time.Now, host: u.Hostname(), lookup: net.DefaultResolver.LookupIPAddr, dial: dialer.DialContext}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.Proxy = nil
	transport.DialContext = v.dialContext
	v.client = &http.Client{
		Timeout:       20 * time.Second,
		Transport:     transport,
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
	return v, nil
}

func (v *Verifier) dialContext(ctx context.Context, network, address string) (net.Conn, error) {
	host, port, err := net.SplitHostPort(address)
	if err != nil || !strings.EqualFold(host, v.host) {
		return nil, errors.New("unexpected website target host")
	}
	addresses, err := v.lookup(ctx, host)
	if err != nil || len(addresses) == 0 {
		return nil, errors.New("website target DNS lookup failed")
	}
	// Validate the whole answer, then dial only those exact addresses. The
	// connection never resolves the name a second time after validation.
	for _, resolved := range addresses {
		ip := resolved.IP
		if ip == nil || resolved.Zone != "" || (v.environment == "local" && !ip.IsLoopback()) || (v.environment == "production" && (!ip.IsGlobalUnicast() || ip.IsPrivate() || ip.IsLoopback() || ip.IsLinkLocalUnicast())) {
			return nil, errors.New("website target resolved outside its environment boundary")
		}
	}
	for _, resolved := range addresses {
		connection, dialErr := v.dial(ctx, network, net.JoinHostPort(resolved.IP.String(), port))
		if dialErr == nil {
			return connection, nil
		}
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
	}
	return nil, errors.New("website target connection failed")
}

type catalogIdentity struct {
	Revision    string `json:"revision"`
	SHA256      string `json:"sha256"`
	Environment string `json:"environment"`
	Path        string `json:"path"`
}

func (v *catalogIdentity) UnmarshalJSON(data []byte) error {
	type wire catalogIdentity
	return strictjson.Object(data, (*wire)(v), "revision", "sha256", "environment", "path")
}

type runtimeFile struct {
	Path      string `json:"path"`
	SHA256    string `json:"sha256"`
	SizeBytes int64  `json:"size_bytes"`
}

func (v *runtimeFile) UnmarshalJSON(data []byte) error {
	type wire runtimeFile
	return strictjson.Object(data, (*wire)(v), "path", "sha256", "size_bytes")
}

type runtimeManifest struct {
	Schema  string          `json:"schema"`
	Catalog catalogIdentity `json:"catalog"`
	Files   []runtimeFile   `json:"files"`
}

func (v *runtimeManifest) UnmarshalJSON(data []byte) error {
	type wire runtimeManifest
	return strictjson.Object(data, (*wire)(v), "schema", "catalog", "files")
}

type catalogManifest struct {
	Schema  string          `json:"schema"`
	State   string          `json:"state"`
	Catalog catalogIdentity `json:"catalog"`
}

func (v *catalogManifest) UnmarshalJSON(data []byte) error {
	type wire catalogManifest
	return strictjson.Object(data, (*wire)(v), "schema", "state", "catalog")
}

func (v *Verifier) read(ctx context.Context, target string, maximum int64) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, v.origin+target+"?aster_publication_check="+rand.Text(), nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Cache-Control", "no-cache, no-store")
	req.Header.Set("Accept", "*/*")
	response, err := v.client.Do(req)
	if err != nil {
		// Do not expose request URLs or an unexpected target's body to operators.
		return nil, errors.New("website target could not be read")
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK || response.ContentLength > maximum {
		return nil, errors.New("website target returned an invalid status or size")
	}
	raw, err := io.ReadAll(io.LimitReader(response.Body, maximum+1))
	if err != nil || int64(len(raw)) > maximum {
		return nil, errors.New("website target returned an incomplete or oversized file")
	}
	return raw, nil
}

func (v *Verifier) Verify(ctx context.Context, snapshot commercial.PublicationSnapshot) (commercial.PublicationEvidence, error) {
	var empty commercial.PublicationEvidence
	if _, err := snapshot.Bytes(); err != nil {
		return empty, err
	}
	if v.environment != snapshot.Catalog.Request.Environment {
		return empty, errors.New("website verifier environment does not match publication")
	}
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	raw, err := v.read(ctx, "/website-release.json", manifestLimit)
	if err != nil {
		return empty, err
	}
	if commercial.ContentDigest(raw) != snapshot.Request.BuildSHA256 {
		return empty, errors.New("website runtime manifest differs from reviewed build")
	}
	var manifest runtimeManifest
	if err = json.Unmarshal(raw, &manifest); err != nil {
		return empty, errors.New("website runtime manifest is invalid")
	}
	identity := catalogIdentity{Revision: snapshot.Catalog.ID, SHA256: snapshot.Catalog.PublicSHA256, Environment: v.environment, Path: "/catalog/" + snapshot.Catalog.ID + "/plans.json"}
	if manifest.Schema != "aster.website-release.v1" || manifest.Catalog != identity || len(manifest.Files) < 3 || len(manifest.Files) > 500 {
		return empty, errors.New("website runtime identity differs from approved catalog")
	}
	seen := map[string]bool{}
	var total int64
	for _, f := range manifest.Files {
		if len(f.Path) > 1024 || !strings.HasPrefix(f.Path, "/") || strings.HasPrefix(f.Path, "//") || path.Clean(f.Path) != f.Path || strings.ContainsAny(f.Path, "\\%?#\r\n\t\x00 :") || f.Path == "/website-release.json" || seen[f.Path] || f.SizeBytes < 0 || f.SizeBytes > fileLimit || len(f.SHA256) != 64 {
			return empty, errors.New("website runtime file list is invalid")
		}
		seen[f.Path] = true
		total += f.SizeBytes
		if total > runtimeLimit {
			return empty, errors.New("website runtime exceeds verification bounds")
		}
	}
	for _, required := range []string{"/index.html", "/catalog-manifest.json", identity.Path} {
		if !seen[required] {
			return empty, fmt.Errorf("website runtime is missing %s", required)
		}
	}
	// Compare the real catalog bytes to the approved canonical digest, never to
	// a locally re-encoded representation of the decoded JSON.
	for _, f := range manifest.Files {
		target := f.Path
		if target == "/index.html" {
			target = "/"
		}
		body, err := v.read(ctx, target, f.SizeBytes)
		if err != nil {
			return empty, err
		}
		if int64(len(body)) != f.SizeBytes || commercial.ContentDigest(body) != f.SHA256 {
			return empty, errors.New("website runtime file differs from reviewed build")
		}
		if f.Path == identity.Path && commercial.ContentDigest(body) != identity.SHA256 {
			return empty, errors.New("website catalog bytes differ from approved catalog")
		}
		if f.Path == "/catalog-manifest.json" {
			var state catalogManifest
			if err := json.Unmarshal(body, &state); err != nil || state.Schema != "aster.website-catalog.v1" || state.State != "configured" || state.Catalog != identity {
				return empty, errors.New("website catalog manifest differs from approved catalog")
			}
		}
	}
	current, err := v.read(ctx, "/website-release.json", manifestLimit)
	if err != nil || !bytes.Equal(current, raw) {
		return empty, errors.New("website changed during verification; reconcile the same publication again")
	}
	return commercial.PublicationEvidence{Environment: v.environment, Origin: v.origin, BuildSHA256: snapshot.Request.BuildSHA256,
		CatalogRevision: identity.Revision, CatalogSHA256: identity.SHA256, ObservedAt: v.now().UTC().Format("2006-01-02T15:04:05.000Z")}, nil
}
