// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package client

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/mixlayer/cli/internal/systemone"
)

// SystemOneURL accepts an API base or an exact systemone endpoint.
func SystemOneURL(base string) (string, error) {
	u, err := url.Parse(base)
	if err != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") || u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return "", fmt.Errorf("endpoint must be an HTTP(S) base URL without credentials, query or fragment")
	}
	u.Path = strings.TrimRight(u.Path, "/")
	if u.Path == "" {
		u.Path = "/v1"
	}
	if !strings.HasSuffix(u.Path, "/systemone") {
		u.Path += "/systemone"
	}
	return u.String(), nil
}

// SystemOne uses the existing config/auth path, with retries limited to explicit
// capacity rejections. It never retries ambiguous transport failures or 5xx.
func (c *Client) SystemOne(ctx context.Context, request systemone.Request) (*systemone.Response, error) {
	if err := request.Validate(); err != nil {
		return nil, err
	}
	target, err := SystemOneURL(c.BaseURL)
	if err != nil {
		return nil, err
	}
	if c.DryRun {
		return nil, fmt.Errorf("systemone dry run must be rendered before transport")
	}
	body, _ := json.Marshal(request)
	var auth string
	if c.SystemOneAuth != nil {
		auth = *c.SystemOneAuth
	} else {
		auth, err = c.authHeader(ctx)
		if err != nil {
			return nil, err
		}
	}
	for attempt := 0; attempt < 4; attempt++ {
		req, err := http.NewRequestWithContext(ctx, http.MethodPost, target, bytes.NewReader(body))
		if err != nil {
			return nil, err
		}
		if c.Config != nil && c.SystemOneAuth == nil {
			for k, v := range c.Config.Headers {
				req.Header.Set(k, v)
			}
		}
		req.Header.Del(BinaryResponseHeader)
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Accept", "application/json")
		req.Header.Set("User-Agent", "mixlayer/1.0.0")
		if auth != "" {
			req.Header.Set("Authorization", auth)
		}
		resp, err := c.HTTPClient.Do(req)
		if err != nil {
			return nil, c.maskError(err, auth)
		}
		data, readErr := io.ReadAll(io.LimitReader(resp.Body, systemone.MaxBody+1))
		resp.Body.Close()
		if readErr != nil {
			return nil, c.maskError(readErr, auth)
		}
		if len(data) > systemone.MaxBody {
			return nil, fmt.Errorf("systemone response exceeds 8 MiB")
		}
		if resp.StatusCode >= 200 && resp.StatusCode < 300 {
			var result systemone.Response
			if err := json.Unmarshal(data, &result); err != nil {
				return nil, fmt.Errorf("invalid systemone response JSON")
			}
			if err := request.ValidateResponse(result); err != nil {
				return nil, err
			}
			return &result, nil
		}
		failure := &APIError{Method: http.MethodPost, Path: "/systemone", StatusCode: resp.StatusCode, Body: c.maskCredentialText(truncateBody(data), auth), RetryAfter: resp.Header.Get("Retry-After")}
		if (resp.StatusCode != 429 && resp.StatusCode != 529) || attempt == 3 {
			return nil, failure
		}
		wait := time.Duration(1<<attempt) * 250 * time.Millisecond
		// A server delay above the retry budget returns the status and hint instead
		// of retrying earlier than requested or blocking indefinitely.
		retryAfter := strings.TrimSpace(failure.RetryAfter)
		if retryAfter != "" && strings.Trim(retryAfter, "0123456789") == "" {
			seconds, err := strconv.ParseUint(retryAfter, 10, 64)
			if err != nil || seconds > 30 {
				return nil, failure
			}
			wait = time.Duration(seconds) * time.Second
		} else if date, err := http.ParseTime(retryAfter); err == nil {
			wait = time.Until(date)
			if wait < 0 {
				wait = 0
			}
		}
		if wait > 30*time.Second {
			return nil, failure
		}
		if err := sleepContext(ctx, wait); err != nil {
			return nil, err
		}
	}
	panic("unreachable")
}
