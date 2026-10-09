// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package upstream

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"strings"
	"time"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/cliutil"
	"github.com/mixlayer/cli/internal/mixapi"
)

type Config struct {
	Name, BaseURL, KeyEnv, Host string
}

var envName = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)

func Resolve(name, baseURL, keyEnv string) (Config, error) {
	c := Config{Name: name}
	switch name {
	case "mixlayer":
		c.BaseURL, c.KeyEnv = "https://mixlayer.ai/v1", "MIXLAYER_API_KEY"
	case "openai":
		c.BaseURL, c.KeyEnv = "https://api.openai.com/v1", "OPENAI_API_KEY"
	case "anthropic":
		// This OpenAI-compatible Anthropic preset must be live-verified before its compatibility is claimed.
		c.BaseURL, c.KeyEnv = "https://api.anthropic.com/v1", "ANTHROPIC_API_KEY"
	case "openrouter":
		c.BaseURL, c.KeyEnv = "https://openrouter.ai/api/v1", "OPENROUTER_API_KEY"
	case "custom":
		if baseURL == "" || keyEnv == "" {
			return c, fmt.Errorf("custom upstream requires --upstream-base-url and --upstream-key-env")
		}
	default:
		return c, fmt.Errorf("unknown upstream %q", name)
	}
	if baseURL != "" {
		if name != "custom" {
			return c, fmt.Errorf("--upstream-base-url requires --upstream custom")
		}
		c.BaseURL = baseURL
	}
	if keyEnv != "" {
		c.KeyEnv = keyEnv
	}
	if !envName.MatchString(c.KeyEnv) {
		return c, fmt.Errorf("invalid key environment variable name %q", c.KeyEnv)
	}
	u, err := url.Parse(c.BaseURL)
	if err != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") || u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return c, fmt.Errorf("invalid upstream base URL")
	}
	c.BaseURL = strings.TrimRight(c.BaseURL, "/")
	c.Host = u.Host
	return c, nil
}

func KeyFromEnv(name string, optional bool) (string, error) {
	if name == "" {
		return "", nil
	}
	if !envName.MatchString(name) {
		return "", fmt.Errorf("invalid key environment variable name %q", name)
	}
	key := os.Getenv(name)
	if key == "" && !optional {
		return "", fmt.Errorf("missing key in environment variable %s", name)
	}
	return key, nil
}

func Chat(ctx context.Context, httpClient *http.Client, baseURL, key, model string, messages []mixapi.Message) (mixapi.ChatResult, error) {
	return ChatWithRequest(ctx, httpClient, baseURL, key, mixapi.ChatRequest{Model: model, Messages: messages})
}

func ChatWithRequest(ctx context.Context, httpClient *http.Client, baseURL, key string, request mixapi.ChatRequest) (mixapi.ChatResult, error) {
	if httpClient == nil {
		httpClient = &http.Client{Timeout: time.Minute}
	}
	// Keep the payload on the reviewed destination even if it redirects.
	requestClient := *httpClient
	requestClient.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	body, err := json.Marshal(request)
	if err != nil {
		return mixapi.ChatResult{}, err
	}
	start := time.Now()
	for attempt := 0; attempt <= 3; attempt++ {
		req, err := http.NewRequestWithContext(ctx, http.MethodPost, strings.TrimRight(baseURL, "/")+"/chat/completions", bytes.NewReader(body))
		if err != nil {
			return mixapi.ChatResult{}, err
		}
		req.Header.Set("Content-Type", "application/json")
		if key != "" {
			req.Header.Set("Authorization", "Bearer "+key)
		}
		resp, err := requestClient.Do(req)
		if err != nil {
			return mixapi.ChatResult{}, err
		}
		raw, readErr := io.ReadAll(io.LimitReader(resp.Body, 8<<20))
		resp.Body.Close()
		if readErr != nil {
			return mixapi.ChatResult{}, readErr
		}
		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			bodyText := string(raw)
			if key != "" {
				bodyText = strings.ReplaceAll(bodyText, key, "[redacted]")
			}
			if len(bodyText) > 4096 {
				bodyText = bodyText[:4096] + "..."
			}
			apiErr := &client.APIError{Method: http.MethodPost, Path: "/chat/completions", StatusCode: resp.StatusCode, Body: bodyText, RetryAfter: resp.Header.Get("Retry-After")}
			if resp.StatusCode == 429 && attempt < 3 {
				wait := cliutil.RetryAfter(resp)
				timer := time.NewTimer(wait)
				select {
				case <-timer.C:
				case <-ctx.Done():
					timer.Stop()
					return mixapi.ChatResult{}, ctx.Err()
				}
				continue
			}
			return mixapi.ChatResult{}, apiErr
		}
		result, err := mixapi.ParseChat(raw)
		if err != nil {
			return mixapi.ChatResult{}, err
		}
		if len(result.Raw) == 0 || result.Answer == "" {
			return mixapi.ChatResult{}, fmt.Errorf("upstream chat response has no answer")
		}
		result.LatencyMS = time.Since(start).Milliseconds()
		return result, nil
	}
	return mixapi.ChatResult{}, fmt.Errorf("upstream chat retry limit exceeded")
}
