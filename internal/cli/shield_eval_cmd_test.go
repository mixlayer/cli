package cli

import (
	"context"
	"errors"
	"path/filepath"
	"testing"

	"github.com/mixlayer/cli/internal/store"
	"github.com/spf13/cobra"
)

func TestShieldEvalContinuesAfterDetectorRecordError(t *testing.T) {
	s, err := store.Open(filepath.Join(t.TempDir(), "eval.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	cmd := &cobra.Command{}
	cmd.SetContext(context.Background())
	calls := 0
	result, err := evaluateDetector(cmd, s, []evalSample{
		{ID: "bad", Text: "Milo Paperboat", Entities: []evalGold{{Kind: "PERSON", Text: "Milo Paperboat"}}},
		{ID: "good", Text: "Zora Quill", Entities: []evalGold{{Kind: "PERSON", Text: "Zora Quill"}}},
	}, detectorRuntime{Name: "local:fake", Call: func(context.Context, string) (string, error) {
		calls++
		if calls == 1 {
			return "", errors.New("bad response")
		}
		return `{"entities":[]}`, nil
	}})
	if err != nil || calls != 2 || result.Errors != 1 || result.Overall.Counts.Gold != 2 || result.Overall.Counts.Found != 1 {
		t.Fatalf("result=%#v calls=%d err=%v", result, calls, err)
	}
}
