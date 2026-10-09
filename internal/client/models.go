// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package client

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
)

// GetModel returns one model object by ID. Mixlayer model IDs contain a
// slash (e.g. "qwen/qwen3.5-9b"), which the gateway's GET /models/{id} route
// does not accept in raw or encoded form, so the model is looked up in the
// GET /models list instead. A missing ID returns an *APIError with status 404.
func (c *Client) GetModel(ctx context.Context, id string) (json.RawMessage, error) {
	data, err := c.Get(ctx, "/models", nil)
	if err != nil {
		return nil, err
	}
	var list struct {
		Data []json.RawMessage `json:"data"`
	}
	if err := json.Unmarshal(data, &list); err != nil {
		return nil, fmt.Errorf("parsing model list: %w", err)
	}
	for _, raw := range list.Data {
		var m struct {
			ID string `json:"id"`
		}
		if json.Unmarshal(raw, &m) == nil && m.ID == id {
			return raw, nil
		}
	}
	return nil, &APIError{Method: http.MethodGet, Path: "/models/" + id, StatusCode: http.StatusNotFound, Body: fmt.Sprintf(`{"error":{"message":"Model '%s' not found"}}`, id)}
}
