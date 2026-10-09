package cli

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/ladder"
	"github.com/mixlayer/cli/internal/pricing"
	"github.com/mixlayer/cli/internal/store"
)

type liveModel struct {
	ModelSKU                      string   `json:"model_sku"`
	PriceMInputTokensUSD          *float64 `json:"price_m_input_tokens_usd"`
	PriceMOutputTokensUSD         *float64 `json:"price_m_output_tokens_usd"`
	PriceMInputCacheReadTokensUSD *float64 `json:"price_m_input_cache_read_tokens_usd"`
	MaxContextLen                 int      `json:"max_context_len"`
	ToolsSupported                bool     `json:"tools_supported"`
	InputModalities               []string `json:"input_modalities"`
	Status                        string   `json:"status"`
	Access                        any      `json:"access"`
	ModelType                     string   `json:"model_type,omitempty"`
}

type liveCatalog struct {
	Models []liveModel `json:"models"`
	Source string      `json:"source"`
}

func (c liveCatalog) price(model string, promptTokens, completionTokens int) (float64, bool) {
	return c.priceWithCache(model, promptTokens, completionTokens, 0)
}

func (c liveCatalog) free(model string) bool {
	for _, item := range c.Models {
		if item.ModelSKU == model && item.PriceMInputTokensUSD != nil && item.PriceMOutputTokensUSD != nil {
			return *item.PriceMInputTokensUSD == 0 && *item.PriceMOutputTokensUSD == 0
		}
	}
	return false
}

func (c liveCatalog) priceWithCache(model string, promptTokens, completionTokens, cacheReadTokens int) (float64, bool) {
	for _, item := range c.Models {
		if item.ModelSKU == model && item.PriceMInputTokensUSD != nil && item.PriceMOutputTokensUSD != nil {
			if cacheReadTokens > 0 && item.PriceMInputCacheReadTokensUSD == nil && *item.PriceMInputTokensUSD != 0 {
				return 0, false
			}
			if cacheReadTokens > promptTokens {
				cacheReadTokens = promptTokens
			}
			cost := float64(promptTokens-cacheReadTokens)/1e6*(*item.PriceMInputTokensUSD) + float64(completionTokens)/1e6*(*item.PriceMOutputTokensUSD)
			if cacheReadTokens > 0 && item.PriceMInputCacheReadTokensUSD != nil {
				cost += float64(cacheReadTokens) / 1e6 * (*item.PriceMInputCacheReadTokensUSD)
			}
			return pricing.RoundUSD(cost), true
		}
	}
	return 0, false
}

func (c liveCatalog) activeRungs() []string {
	models := append([]liveModel(nil), c.Models...)
	sort.Slice(models, func(i, j int) bool {
		if models[i].PriceMOutputTokensUSD == nil {
			return false
		}
		if models[j].PriceMOutputTokensUSD == nil {
			return true
		}
		return *models[i].PriceMOutputTokensUSD < *models[j].PriceMOutputTokensUSD
	})
	var out []string
	for _, model := range models {
		if strings.EqualFold(model.Status, "active") && isGenerationModel(model) {
			out = append(out, model.ModelSKU)
		}
	}
	return out
}

func (c liveCatalog) frontier() string {
	var chosen string
	maxPrice := -1.0
	for _, model := range c.Models {
		if strings.EqualFold(model.Status, "active") && isGenerationModel(model) && model.PriceMOutputTokensUSD != nil && *model.PriceMOutputTokensUSD > maxPrice {
			chosen, maxPrice = model.ModelSKU, *model.PriceMOutputTokensUSD
		}
	}
	return chosen
}

func isGenerationModel(model liveModel) bool {
	return !strings.Contains(strings.ToLower(model.ModelType), "embed") && !strings.Contains(strings.ToLower(model.ModelSKU), "embed")
}

func (c liveCatalog) defaultModel() string {
	if frontier := c.frontier(); frontier != "" {
		return frontier
	}
	if c.Source == "ids_only" {
		for _, model := range c.Models {
			if model.ModelSKU == defaultFrontierModel {
				return model.ModelSKU
			}
		}
		if len(c.Models) > 0 {
			return c.Models[0].ModelSKU
		}
	}
	return defaultFrontierModel
}

func rungsForCatalog(spec string, catalog liveCatalog) []string {
	if strings.TrimSpace(spec) == "" || strings.EqualFold(spec, "all") {
		if rungs := catalog.activeRungs(); len(rungs) > 0 {
			return rungs
		}
		if catalog.Source == "ids_only" {
			rungs := make([]string, 0, len(catalog.Models))
			for _, model := range catalog.Models {
				rungs = append(rungs, model.ModelSKU)
			}
			return rungs
		}
	}
	return ladder.Rungs(spec)
}

func applyCatalogCosts(results []ladder.Result, catalog liveCatalog) {
	for i := range results {
		if results[i].Error != "" || results[i].Answer == "" || (!results[i].UsageReported && !catalog.free(results[i].Model)) {
			results[i].CostUnknown = true
			continue
		}
		if cost, ok := catalog.priceWithCache(results[i].Model, results[i].PromptTokens, results[i].CompletionTokens, results[i].CacheReadTokens); ok {
			results[i].CostUSD, results[i].CostUnknown = cost, false
		} else {
			results[i].CostUnknown = true
		}
	}
}

func offlineCatalog() liveCatalog {
	models := make([]liveModel, 0, len(pricing.DefaultRungs))
	for _, sku := range pricing.DefaultRungs {
		models = append(models, liveModel{ModelSKU: sku, Status: "offline fallback"})
	}
	return liveCatalog{Models: models, Source: "offline_fallback"}
}

func loadLiveCatalog(ctx context.Context, flags *rootFlags, s *store.Store) (liveCatalog, error) {
	c, err := flags.newClient()
	if err != nil {
		return offlineCatalog(), err
	}
	base := os.Getenv("MIXLAYER_PLATFORM_URL")
	if base == "" {
		base = "https://api.mixlayer.com"
	}
	base = strings.TrimRight(base, "/")
	parsed, err := url.Parse(base)
	if err != nil || parsed.Host == "" || (parsed.Scheme != "http" && parsed.Scheme != "https") || parsed.User != nil {
		return offlineCatalog(), fmt.Errorf("invalid MIXLAYER_PLATFORM_URL")
	}
	auth := c.Config.AuthHeader()
	if auth == "" {
		return offlineCatalog(), fmt.Errorf("MIXLAYER_API_KEY is not configured")
	}
	fingerprint := sha256.Sum256([]byte(auth))
	scope := fmt.Sprintf("%s:%x", base, fingerprint[:8])
	if raw, ok, err := s.CachedLiveCatalog(ctx, scope, time.Hour); err != nil {
		return offlineCatalog(), err
	} else if ok {
		var cached liveCatalog
		if json.Unmarshal(raw, &cached) == nil && len(cached.Models) > 0 {
			return cached, nil
		}
	}
	timeout := flags.timeout
	if timeout <= 0 || timeout > 5*time.Second {
		timeout = 5 * time.Second
	}
	httpClient := &http.Client{Timeout: timeout, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	get := func(path string) (json.RawMessage, error) {
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, base+path, nil)
		if err != nil {
			return nil, err
		}
		if auth != "" {
			req.Header.Set("Authorization", auth)
		}
		resp, err := httpClient.Do(req)
		if err != nil {
			return nil, err
		}
		defer resp.Body.Close()
		body, err := io.ReadAll(io.LimitReader(resp.Body, 4<<20))
		if err != nil {
			return nil, err
		}
		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			return nil, &client.APIError{Method: http.MethodGet, Path: path, StatusCode: resp.StatusCode, Body: string(body), RetryAfter: resp.Header.Get("Retry-After")}
		}
		return body, nil
	}
	var catalog liveCatalog
	organizations, orgErr := get("/v1/organizations")
	if orgErr == nil {
		var envelope any
		if json.Unmarshal(organizations, &envelope) == nil {
			for _, org := range catalogItems(envelope) {
				orgID := stringField(org, "id", "slug", "organization_id")
				if orgID == "" {
					continue
				}
				raw, err := get("/v1/organizations/" + url.PathEscape(orgID) + "/models")
				if err == nil {
					catalog = parseLiveCatalog(raw, "organization")
					if len(catalog.Models) > 0 {
						break
					}
				}
			}
		}
	}
	if len(catalog.Models) == 0 {
		raw, err := get("/v1/models")
		if err != nil {
			if orgErr != nil {
				return offlineCatalog(), fmt.Errorf("live catalog: %w; fallback: %w", orgErr, err)
			}
			return offlineCatalog(), err
		}
		catalog = parseLiveCatalog(raw, "ids_only")
	}
	if len(catalog.Models) == 0 {
		return offlineCatalog(), fmt.Errorf("live catalog returned no models")
	}
	encoded, err := json.Marshal(catalog)
	if err == nil {
		_ = s.SaveLiveCatalog(ctx, scope, encoded)
	}
	return catalog, nil
}

func parseLiveCatalog(raw json.RawMessage, source string) liveCatalog {
	var decoded any
	if json.Unmarshal(raw, &decoded) != nil {
		return liveCatalog{Source: source}
	}
	models := []liveModel{}
	for _, item := range catalogItems(decoded) {
		sku := stringField(item, "model_sku", "id", "model", "name")
		if sku == "" {
			continue
		}
		model := liveModel{ModelSKU: sku, MaxContextLen: intField(item, "max_context_len", "context_window"), ToolsSupported: boolField(item, false, "tools_supported"), Status: stringField(item, "status"), Access: item["access"], ModelType: stringField(item, "model_type", "type")}
		if model.Status == "" && source == "ids_only" {
			model.Status = "unknown"
		}
		if modalities, ok := item["input_modalities"].([]any); ok {
			for _, value := range modalities {
				if text, ok := value.(string); ok {
					model.InputModalities = append(model.InputModalities, text)
				}
			}
		}
		if source == "organization" {
			model.PriceMInputTokensUSD = numberPointer(item, "price_m_input_tokens_usd")
			model.PriceMOutputTokensUSD = numberPointer(item, "price_m_output_tokens_usd")
			model.PriceMInputCacheReadTokensUSD = numberPointer(item, "price_m_input_cache_read_tokens_usd")
		}
		models = append(models, model)
	}
	return liveCatalog{Models: models, Source: source}
}

func numberPointer(item map[string]any, key string) *float64 {
	value, ok := item[key]
	if !ok || value == nil {
		return nil
	}
	switch n := value.(type) {
	case float64:
		return &n
	case json.Number:
		v, err := n.Float64()
		if err == nil {
			return &v
		}
	case string:
		v, err := strconv.ParseFloat(n, 64)
		if err == nil {
			return &v
		}
	}
	return nil
}
