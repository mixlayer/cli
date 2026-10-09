// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/mixapi"
	"github.com/mixlayer/cli/internal/shield"
	"github.com/mixlayer/cli/internal/store"
	"github.com/mixlayer/cli/internal/upstream"
	"github.com/spf13/cobra"
)

const placeholderInstruction = "Placeholders matching <KIND>_<hex> are stand-ins for private values. Repeat each placeholder exactly. Never alter or expand it, and never guess the real value."
const mixlayerDetectorNotice = "raw text is sent to Mixlayer for detection; the frontier upstream receives masked text only"

type detectorFlags struct {
	Name, BaseURL, KeyEnv, Fallback string
	Timeout                         time.Duration
}

func (d *detectorFlags) bind(cmd *cobra.Command) {
	cmd.Flags().StringVar(&d.Name, "detector", "regex", "Detector: regex, local:<model>, or mixlayer:<model>")
	cmd.Flags().StringVar(&d.BaseURL, "detector-base-url", "http://localhost:11434/v1", "OpenAI-compatible local detector base URL")
	cmd.Flags().StringVar(&d.KeyEnv, "detector-key-env", "", "Environment variable name for detector key")
	cmd.Flags().StringVar(&d.Fallback, "detector-fallback", "", "Use regex if a model detector fails (regex)")
	cmd.Flags().DurationVar(&d.Timeout, "detector-timeout", time.Minute, "Maximum time for each detector request")
}

type detectorRuntime struct {
	Name, Host, Notice string
	Call               shield.ModelCall
	Fallback           bool
	IgnoredSpans       *int
	Usage              *detectorUsage
	// ExtraTerms are run-scoped mask terms (for example from --mask-columns).
	// They are applied like dictionary terms but never saved.
	ExtraTerms []store.DictionaryTerm
}

type detectorUsage struct {
	PromptTokens, CompletionTokens, CacheReadTokens, Calls int
	UnknownUsage                                           bool
}

func (d detectorFlags) runtime(cmd *cobra.Command, flags *rootFlags) (detectorRuntime, error) {
	if d.Fallback != "" && d.Fallback != "regex" {
		return detectorRuntime{}, usageErr(fmt.Errorf("--detector-fallback must be regex"))
	}
	ignored := 0
	usage := &detectorUsage{}
	r := detectorRuntime{Name: d.Name, Fallback: d.Fallback == "regex", IgnoredSpans: &ignored, Usage: usage}
	if d.Name == "regex" {
		return r, nil
	}
	parts := strings.SplitN(d.Name, ":", 2)
	if len(parts) != 2 || parts[1] == "" {
		return r, usageErr(fmt.Errorf("invalid --detector %q", d.Name))
	}
	model := parts[1]
	timeout := d.Timeout
	if timeout <= 0 {
		timeout = time.Minute
	}
	if flags.timeout > 0 && flags.timeout < timeout {
		timeout = flags.timeout
	}
	callWith := func(ctx context.Context, prompt string, local bool, send func(context.Context, mixapi.ChatRequest) (mixapi.ChatResult, error)) (string, error) {
		instruction, text, ok := strings.Cut(prompt, " Text: ")
		if !ok {
			return "", fmt.Errorf("invalid detector prompt")
		}
		if local {
			instruction = "/no_think\n" + instruction
		}
		zero := 0.0
		req := mixapi.ChatRequest{Model: model, Messages: []mixapi.Message{{Role: "system", Content: instruction}, {Role: "user", Content: "Text: " + text}}, Temperature: &zero, ReasoningEffort: "none", ResponseFormat: json.RawMessage(`{"type":"json_object"}`)}
		requestCtx, cancel := context.WithTimeout(ctx, timeout)
		defer cancel()
		res, err := send(requestCtx, req)
		var apiError *client.APIError
		if err != nil && errors.As(err, &apiError) && apiError.StatusCode == 400 {
			req.ResponseFormat = nil
			res, err = send(requestCtx, req)
		}
		if err == nil {
			usage.PromptTokens += res.PromptTokens
			usage.CompletionTokens += res.CompletionTokens
			usage.CacheReadTokens += res.CacheReadTokens
			usage.Calls++
			if !res.UsageReported {
				usage.UnknownUsage = true
			}
		}
		return res.Answer, err
	}
	switch parts[0] {
	case "local":
		u, err := url.Parse(d.BaseURL)
		if err != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") || u.User != nil ||
			(u.Hostname() != "localhost" && u.Hostname() != "127.0.0.1" && u.Hostname() != "::1") {
			return r, usageErr(fmt.Errorf("invalid --detector-base-url"))
		}
		r.Host = u.Host
		key, err := upstream.KeyFromEnv(d.KeyEnv, false)
		if err != nil {
			return r, authErr(err)
		}
		r.Call = func(ctx context.Context, prompt string) (string, error) {
			return callWith(ctx, prompt, true, func(ctx context.Context, req mixapi.ChatRequest) (mixapi.ChatResult, error) {
				return upstream.ChatWithRequest(ctx, &http.Client{Timeout: timeout}, d.BaseURL, key, req)
			})
		}
	case "mixlayer":
		c, err := flags.newClient()
		if err != nil {
			return r, err
		}
		u, err := url.Parse(c.RequestBaseURL())
		if err != nil {
			return r, err
		}
		r.Host = u.Host
		r.Notice = mixlayerDetectorNotice
		if !flags.asJSON && !flags.agent {
			fmt.Fprintln(cmd.ErrOrStderr(), r.Notice)
		}
		if d.KeyEnv != "" {
			key, err := upstream.KeyFromEnv(d.KeyEnv, false)
			if err != nil {
				return r, authErr(err)
			}
			r.Call = func(ctx context.Context, prompt string) (string, error) {
				return callWith(ctx, prompt, false, func(ctx context.Context, req mixapi.ChatRequest) (mixapi.ChatResult, error) {
					return upstream.ChatWithRequest(ctx, &http.Client{Timeout: timeout}, c.RequestBaseURL(), key, req)
				})
			}
		} else {
			r.Call = func(ctx context.Context, prompt string) (string, error) {
				return callWith(ctx, prompt, false, func(ctx context.Context, req mixapi.ChatRequest) (mixapi.ChatResult, error) {
					return mixapi.Chat(ctx, c, req)
				})
			}
		}
	default:
		return r, usageErr(fmt.Errorf("invalid --detector %q", d.Name))
	}
	return r, nil
}

func detectAndRedact(ctx context.Context, s *store.Store, text string, reveal bool, detector detectorRuntime) (shield.RedactionResult, bool, error) {
	terms, err := s.DictionaryTerms(ctx)
	if err != nil {
		return shield.RedactionResult{}, false, err
	}
	terms = append(terms, detector.ExtraTerms...)
	entities, fallback, ignored, err := shield.DetectWithStats(ctx, text, terms, detector.Call, detector.Fallback)
	if detector.IgnoredSpans != nil {
		*detector.IgnoredSpans += ignored
	}
	if err != nil {
		return shield.RedactionResult{}, false, err
	}
	res, err := shield.RedactEntities(ctx, s, text, entities, reveal)
	return res, fallback, err
}

func saveShieldChat(ctx context.Context, flags *rootFlags, s *store.Store, cfg upstream.Config, model, payload string, showThinking bool, detectorCost *float64, detectorCostUnknown bool) (store.RunRecord, error) {
	messages := []mixapi.Message{{Role: "system", Content: placeholderInstruction}, {Role: "user", Content: payload}}
	var res mixapi.ChatResult
	var err error
	if cfg.Name == "mixlayer" && cfg.KeyEnv == "MIXLAYER_API_KEY" {
		c, clientErr := flags.newClient()
		if clientErr != nil {
			return store.RunRecord{}, clientErr
		}
		thinking := showThinking
		res, err = mixapi.Chat(ctx, c, mixapi.ChatRequest{Model: model, Messages: messages, Thinking: &thinking})
	} else {
		key, keyErr := upstream.KeyFromEnv(cfg.KeyEnv, false)
		if keyErr != nil {
			return store.RunRecord{}, authErr(keyErr)
		}
		res, err = upstream.Chat(ctx, &http.Client{Timeout: flags.timeout}, cfg.BaseURL, key, model, messages)
	}
	if err != nil {
		return store.RunRecord{}, classifyAPIError(err, flags)
	}
	params, _ := json.Marshal(map[string]any{"model": model, "upstream": cfg.Name, "reasoning": showThinking})
	run := store.RunRecord{ID: store.NewID("run"), Command: "shield ask", Prompt: payload, Answer: res.Answer, Reasoning: res.Reasoning,
		Model: model, ParamsJSON: params, RawJSON: res.Raw, PromptTokens: res.PromptTokens, CompletionTokens: res.CompletionTokens,
		TotalTokens: res.TotalTokens, LatencyMS: res.LatencyMS, CostUnknown: true}
	if cfg.Name == "mixlayer" {
		catalog, _ := loadLiveCatalog(ctx, flags, s)
		run.CatalogSource = catalog.Source
		if cost, ok := catalog.priceWithCache(model, res.PromptTokens, res.CompletionTokens, res.CacheReadTokens); ok && (res.UsageReported || catalog.free(model)) {
			run.CostUSD, run.CostUnknown = cost, false
		}
	}
	if detectorCostUnknown {
		run.CostUnknown = true
	} else if detectorCost != nil && !run.CostUnknown {
		run.CostUSD += *detectorCost
	}
	if err := s.SaveRun(ctx, run); err != nil {
		return store.RunRecord{}, err
	}
	return run, nil
}
