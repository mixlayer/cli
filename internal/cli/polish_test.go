package cli

import (
	"bytes"
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/mixlayer/cli/internal/store"
	"github.com/spf13/cobra"
)

func TestAgentDryRunPreviewsCustomCommandsWithoutNetwork(t *testing.T) {
	flags := &rootFlags{dryRun: true, agent: true, asJSON: true, timeout: time.Second}
	db := filepath.Join(t.TempDir(), "runs.db")
	s, err := store.Open(db)
	if err != nil {
		t.Fatal(err)
	}
	if err := s.SaveRun(context.Background(), store.RunRecord{ID: "run-test", Command: "ask", Prompt: "local prompt", Model: "fake/model"}); err != nil {
		t.Fatal(err)
	}
	s.Close()
	cases := []struct {
		name string
		cmd  *cobra.Command
		args []string
	}{
		{"ask", newAskCmd(flags), []string{"Hello"}},
		{"replay", newReplayCmd(flags), []string{"run-test", "--db", db}},
		{"ladder", newLadderCmd(flags), []string{"Hello"}},
		{"escalate", newEscalateCmd(flags), []string{"Hello"}},
		{"council", newCouncilCmd(flags), []string{"Hello"}},
		{"compare", newCompareCmd(flags), []string{"Hello"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var out bytes.Buffer
			tc.cmd.SetOut(&out)
			tc.cmd.SetArgs(tc.args)
			if err := tc.cmd.Execute(); err != nil {
				t.Fatal(err)
			}
			var preview map[string]any
			if err := json.Unmarshal(out.Bytes(), &preview); err != nil {
				t.Fatalf("output=%s err=%v", out.String(), err)
			}
			if preview["dry_run"] != true || preview["endpoint"] != "/chat/completions" || preview["models"] == nil || (preview["messages"] == nil && preview["prompt"] == nil && preview["prompt_summary"] == nil) {
				t.Fatalf("preview=%#v", preview)
			}
		})
	}
}

func TestWhichRoutesAuthAndModelComparison(t *testing.T) {
	for query, want := range map[string]string{"check auth": "doctor", "compare models": "compare"} {
		matches := rankWhich(whichIndex, query, 3)
		if len(matches) == 0 || matches[0].Entry.Command != want {
			t.Fatalf("%q = %#v, want %s", query, matches, want)
		}
	}
}

func TestVersionAgentAndExportMissingArgs(t *testing.T) {
	flags := &rootFlags{agent: true, asJSON: true}
	var out bytes.Buffer
	versionCmd := newVersionCmd(flags)
	versionCmd.SetOut(&out)
	if err := versionCmd.Execute(); err != nil {
		t.Fatal(err)
	}
	var versionResult map[string]any
	if err := json.Unmarshal(out.Bytes(), &versionResult); err != nil || versionResult["version"] != version {
		t.Fatalf("version=%s err=%v", out.String(), err)
	}

}
