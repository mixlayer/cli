package cli

import (
	"encoding/json"
	"math"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/ladder"
)

func TestLiveCatalogPricingUsedForAskCost(t *testing.T) {
	var orgModelsCalls int
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/organizations":
			_, _ = w.Write([]byte(`{"organizations":[{"id":"org-test"}]}`))
		case "/v1/organizations/org-test/models":
			orgModelsCalls++
			_, _ = w.Write([]byte(`{"models":[{"model_sku":"fake/live","price_m_input_tokens_usd":2,"price_m_output_tokens_usd":4,"price_m_input_cache_read_tokens_usd":0,"max_context_len":8192,"tools_supported":true,"input_modalities":["text"],"status":"active","access":"allowed"}]}`))
		case "/v1/chat/completions":
			_, _ = w.Write([]byte(`{"choices":[{"message":{"content":"done"}}],"usage":{"prompt_tokens":1000,"completion_tokens":500,"total_tokens":1500}}`))
		default:
			t.Errorf("unexpected path %s", r.URL.Path)
			http.NotFound(w, r)
		}
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_BASE_URL", server.URL+"/v1")
	t.Setenv("MIXLAYER_PLATFORM_URL", server.URL)
	t.Setenv("MIXLAYER_DATA_DIR", t.TempDir())
	t.Setenv("MIXLAYER_CONFIG_DIR", t.TempDir())
	flags := &rootFlags{timeout: 3 * time.Second, asJSON: true}
	dbPath := filepath.Join(t.TempDir(), "data.db")
	out, err := runCommand(t, newAskCmd(flags), "Hello", "--db", dbPath)
	if err != nil {
		t.Fatal(err)
	}
	var run map[string]any
	if err := json.Unmarshal([]byte(out), &run); err != nil {
		t.Fatal(err)
	}
	if run["model"] != "fake/live" || run["cost_usd"] != 0.004 || run["catalog_source"] != "organization" || orgModelsCalls != 1 {
		t.Fatalf("run=%#v catalog calls=%d", run, orgModelsCalls)
	}
}

func TestMixlayerDetectorCostIncludedInShieldReceipt(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/organizations":
			_, _ = w.Write([]byte(`{"organizations":[{"id":"org-test"}]}`))
		case "/v1/organizations/org-test/models":
			_, _ = w.Write([]byte(`{"models":[{"model_sku":"fake/live","price_m_input_tokens_usd":1,"price_m_output_tokens_usd":2,"status":"active"}]}`))
		case "/v1/chat/completions":
			var req struct {
				Messages []struct {
					Content string `json:"content"`
				} `json:"messages"`
			}
			if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
				t.Error(err)
			}
			if strings.Contains(req.Messages[0].Content, "Return ONLY strict JSON") {
				response := `{"entities":[]}`
				if strings.Contains(req.Messages[1].Content, "milo paperboat") {
					response = `{"entities":[{"kind":"PERSON","text":"milo paperboat"}]}`
				}
				_ = json.NewEncoder(w).Encode(map[string]any{"choices": []map[string]any{{"message": map[string]string{"content": response}}}, "usage": map[string]int{"prompt_tokens": 100, "completion_tokens": 50, "total_tokens": 150}})
			} else {
				_, _ = w.Write([]byte(`{"choices":[{"message":{"content":"done"}}],"usage":{"prompt_tokens":1000,"completion_tokens":500,"total_tokens":1500}}`))
			}
		default:
			t.Errorf("unexpected path %s", r.URL.Path)
			http.NotFound(w, r)
		}
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_BASE_URL", server.URL+"/v1")
	t.Setenv("MIXLAYER_PLATFORM_URL", server.URL)
	t.Setenv("MIXLAYER_DATA_DIR", t.TempDir())
	t.Setenv("MIXLAYER_CONFIG_DIR", t.TempDir())
	result, err := executeShieldAsk(t, "Summarize milo paperboat", "--detector", "mixlayer:fake/live", "--model", "fake/live", "--db", filepath.Join(t.TempDir(), "data.db"))
	if err != nil {
		t.Fatal(err)
	}
	audit := result["audit"].(map[string]any)
	run := result["run"].(map[string]any)
	if math.Abs(audit["detector_cost_usd"].(float64)-0.0004) > 1e-10 || math.Abs(audit["cost_usd"].(float64)-0.0024) > 1e-10 || math.Abs(run["cost_usd"].(float64)-0.0024) > 1e-10 {
		t.Fatalf("audit=%#v run=%#v", audit, run)
	}
}

func TestLiveCatalogSortsActiveRungsAndKeepsUnknownUnpriced(t *testing.T) {
	cheap, costly := 0.0, 3.0
	catalog := liveCatalog{Source: "organization", Models: []liveModel{
		{ModelSKU: "expensive", PriceMInputTokensUSD: &costly, PriceMOutputTokensUSD: &costly, Status: "active"},
		{ModelSKU: "inactive", PriceMInputTokensUSD: &cheap, PriceMOutputTokensUSD: &cheap, Status: "inactive"},
		{ModelSKU: "free", PriceMInputTokensUSD: &cheap, PriceMOutputTokensUSD: &cheap, Status: "active"},
		{ModelSKU: "unknown", Status: "active"},
	}}
	rungs := catalog.activeRungs()
	if len(rungs) != 3 || rungs[0] != "free" || rungs[1] != "expensive" || catalog.frontier() != "expensive" {
		t.Fatalf("rungs=%#v frontier=%s", rungs, catalog.frontier())
	}
	if price, ok := catalog.price("free", 100, 100); !ok || price != 0 {
		t.Fatalf("free price=%v known=%v", price, ok)
	}
	if _, ok := catalog.price("unknown", 100, 100); ok {
		t.Fatal("unpriced model became a free model")
	}
}

func TestLiveCatalogIDsOnlyFallbackPreservesAvailableModels(t *testing.T) {
	catalog := parseLiveCatalog([]byte(`{"models":["fake/one","fake/two"]}`), "ids_only")
	rungs := rungsForCatalog("all", catalog)
	if len(rungs) != 2 || rungs[0] != "fake/one" || rungs[1] != "fake/two" || catalog.defaultModel() != "fake/one" {
		t.Fatalf("catalog=%#v rungs=%#v", catalog, rungs)
	}
}

func TestLiveCatalogUsesCacheReadPriceWhenReported(t *testing.T) {
	input, output, cacheRead := 2.0, 4.0, 0.5
	catalog := liveCatalog{Models: []liveModel{{ModelSKU: "cached", PriceMInputTokensUSD: &input, PriceMOutputTokensUSD: &output, PriceMInputCacheReadTokensUSD: &cacheRead}}}
	cost, ok := catalog.priceWithCache("cached", 1000, 500, 400)
	want := (600*input + 400*cacheRead + 500*output) / 1e6
	if !ok || math.Abs(cost-want) > 1e-12 {
		t.Fatalf("cost=%f known=%v want=%f", cost, ok, want)
	}
}

func TestLiveCatalogMissingUsageRemainsUnknown(t *testing.T) {
	paid := 1.0
	catalog := liveCatalog{Models: []liveModel{{ModelSKU: "paid", PriceMInputTokensUSD: &paid, PriceMOutputTokensUSD: &paid}}}
	results := []ladder.Result{{Model: "paid", Answer: "ok"}}
	applyCatalogCosts(results, catalog)
	if !results[0].CostUnknown {
		t.Fatal("missing token usage was priced as zero")
	}
}

func TestModelsListLivePricesUsesOrganizationCatalog(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/organizations":
			_, _ = w.Write([]byte(`{"organizations":[{"id":"org-test"}]}`))
		case "/v1/organizations/org-test/models":
			_, _ = w.Write([]byte(`{"models":[{"model_sku":"fake/live","price_m_input_tokens_usd":0,"price_m_output_tokens_usd":0,"status":"active","tools_supported":true,"input_modalities":["text"]}]}`))
		default:
			t.Errorf("unexpected path %s", r.URL.Path)
		}
	}))
	defer server.Close()
	t.Setenv("MIXLAYER_API_KEY", "dummy")
	t.Setenv("MIXLAYER_PLATFORM_URL", server.URL)
	t.Setenv("MIXLAYER_DATA_DIR", t.TempDir())
	t.Setenv("MIXLAYER_CONFIG_DIR", t.TempDir())
	flags := &rootFlags{timeout: 3 * time.Second, asJSON: true}
	out, err := runCommand(t, newModelsListCmd(flags), "--live-prices", "--db", filepath.Join(t.TempDir(), "catalog.db"))
	if err != nil {
		t.Fatal(err)
	}
	var payload struct {
		Source string `json:"source"`
		Models []struct {
			ModelSKU string   `json:"model_sku"`
			Output   *float64 `json:"price_m_output_tokens_usd"`
		} `json:"models"`
	}
	if err := json.Unmarshal([]byte(out), &payload); err != nil || payload.Source != "organization" || len(payload.Models) != 1 || payload.Models[0].ModelSKU != "fake/live" || payload.Models[0].Output == nil || *payload.Models[0].Output != 0 {
		t.Fatalf("payload=%#v err=%v", payload, err)
	}
}
