package client

import (
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/config"
)

func TestResponseCacheKeyScopesBaseURLAndKeyFingerprint(t *testing.T) {
	first := New(&config.Config{BaseURL: "https://one.example/v1", MixlayerApiKey: "dummy-one"}, time.Second, 0)
	changedKey := New(&config.Config{BaseURL: "https://one.example/v1", MixlayerApiKey: "dummy-two"}, time.Second, 0)
	changedURL := New(&config.Config{BaseURL: "https://two.example/v1", MixlayerApiKey: "dummy-one"}, time.Second, 0)
	a, b, c := first.cacheKey("/models", nil), changedKey.cacheKey("/models", nil), changedURL.cacheKey("/models", nil)
	if a == b || a == c || b == c {
		t.Fatalf("cache keys failed to isolate URL or key: %s %s %s", a, b, c)
	}
}
