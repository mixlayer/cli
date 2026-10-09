// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"context"
	"encoding/json"

	"github.com/mixlayer/cli/internal/client"
)

// liveRead performs a GET against the Mixlayer API and labels the result as
// live data for the JSON envelope.
func liveRead(ctx context.Context, c *client.Client, path string, params map[string]string) (json.RawMessage, DataProvenance, error) {
	data, err := c.Get(ctx, path, params)
	if err != nil {
		return nil, DataProvenance{}, err
	}
	return data, DataProvenance{Source: "live"}, nil
}
