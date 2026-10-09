// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package cli

import (
	"errors"
	"fmt"
	"io"
	"math"
	"net/url"
	"os"
	"regexp"
	"sort"
	"strconv"
	"strings"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/systemone"
	"github.com/spf13/cobra"
)

const decisionGateExitCode = 9

type decideOptions struct {
	model, endpoint, endpointKeyEnv, questions string
}

func (o *decideOptions) addFlags(cmd *cobra.Command) {
	cmd.Flags().StringVar(&o.model, "model", "", "Classification model (falls back to config classification_model; no default)")
	cmd.Flags().StringVar(&o.endpoint, "endpoint", "", "Override API base URL or exact systemone URL for self-hosted Jini (the Mixlayer key is not sent to other hosts)")
	cmd.Flags().StringVar(&o.endpointKeyEnv, "endpoint-key-env", "", "Name of an env var holding a bearer key for a foreign --endpoint")
	cmd.Flags().StringVar(&o.questions, "questions", "", "Systemone questions map JSON file, or - for stdin")
}

func (o decideOptions) client(flags *rootFlags) (*client.Client, string, error) {
	c, err := flags.newClient()
	if err != nil {
		return nil, "", err
	}
	model := o.model
	if model == "" {
		model = c.Config.ClassificationModel
	}
	if strings.TrimSpace(model) == "" {
		return nil, "", usageErr(fmt.Errorf("classification model is required\nhint: set --model or classification_model in config.toml"))
	}
	if o.endpoint != "" {
		original, _ := url.Parse(c.BaseURL)
		target, err := url.Parse(o.endpoint)
		if err != nil {
			return nil, "", usageErr(fmt.Errorf("invalid --endpoint: %w", err))
		}
		// Never forward the Mixlayer credential or config headers to another host.
		if original == nil || !strings.EqualFold(original.Host, target.Host) {
			auth := ""
			if o.endpointKeyEnv != "" {
				v := strings.TrimSpace(os.Getenv(o.endpointKeyEnv))
				if v == "" {
					return nil, "", usageErr(fmt.Errorf("--endpoint-key-env %s is empty or unset", o.endpointKeyEnv))
				}
				auth = "Bearer " + v
			}
			c.SystemOneAuth = &auth
		}
		c.BaseURL = o.endpoint
	}
	if _, err := client.SystemOneURL(c.BaseURL); err != nil {
		return nil, "", usageErr(err)
	}
	return c, model, nil
}

func readDecisionInput(cmd *cobra.Command, path string) ([]byte, error) {
	r := cmd.InOrStdin()
	if path != "-" {
		f, err := os.Open(path)
		if err != nil {
			return nil, err
		}
		defer f.Close()
		r = f
	}
	data, err := io.ReadAll(io.LimitReader(r, systemone.MaxBody+1))
	if err != nil {
		return nil, err
	}
	if len(data) > systemone.MaxBody {
		return nil, fmt.Errorf("input exceeds 8 MiB")
	}
	return data, nil
}

func decisionQuestions(cmd *cobra.Command, path, inline string, nouls []string) (systemone.Questions, error) {
	questions := systemone.Questions{}
	if path != "" && inline != "" {
		return nil, fmt.Errorf("use only one of --questions and --questions-json")
	}
	if path != "" || inline != "" {
		data := []byte(inline)
		if path != "" {
			var err error
			data, err = readDecisionInput(cmd, path)
			if err != nil {
				return nil, err
			}
		}
		if len(data) > systemone.MaxBody {
			return nil, fmt.Errorf("questions exceed 8 MiB")
		}
		var err error
		questions, err = systemone.ParseQuestions(data)
		if err != nil {
			return nil, err
		}
	}
	for _, spec := range nouls {
		id, instructions, ok := strings.Cut(spec, "=")
		if !ok {
			return nil, fmt.Errorf("--noul expects id=instructions")
		}
		if _, ok := questions[id]; ok {
			return nil, fmt.Errorf("duplicate question ID %q", id)
		}
		questions[id] = systemone.Question{Type: "noul", Instructions: systemone.JSON(instructions)}
	}
	return questions, nil
}

func newDecideCmd(flags *rootFlags) *cobra.Command {
	var o decideOptions
	var statePath, stateText, inlineQuestions string
	var nouls, failIf []string
	cmd := &cobra.Command{
		Use: "decide", Short: "Evaluate typed Jini decisions and gate CI on the answers",
		Long: "Evaluate noul, choice and score questions through POST /v1/systemone.\nJSON is returned with --agent or --json; human output is a compact table.\nA matching --fail-if condition exits 9 after printing the response.\nOther exits: 2 invalid input/HTTP 422, 4 auth/permission, 5 API/capacity (529),\n7 rate limit (429), 8 billing (402), 10 configuration.\nRetries: at most three retries for 429/529; Retry-After above 30s returns immediately.\nNoul values are yes scores, not calibrated confidence probabilities.",
		Example: `  mixlayer decide --model <classification-model> --state-text "Two charges for one order" --noul 'dup=Is this a duplicate charge?' --fail-if 'dup>0.8' --agent
  mixlayer decide --model <classification-model> --state facts.json --questions questions.json --dry-run --agent`,
		Annotations: map[string]string{"mcp:hidden": "true", "cli:no-error-path-probe": "true", "mcp:read-only": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) > 0 {
				return usageErr(fmt.Errorf("decide takes flags only; see --help"))
			}
			if cmd.Flags().Changed("state") == cmd.Flags().Changed("state-text") {
				return usageErr(fmt.Errorf("provide exactly one of --state or --state-text"))
			}
			if statePath == "-" && o.questions == "-" {
				return usageErr(fmt.Errorf("state and questions cannot both read stdin"))
			}
			questions, err := decisionQuestions(cmd, o.questions, inlineQuestions, nouls)
			if err != nil {
				return usageErr(err)
			}
			gates, err := parseDecisionGates(failIf, questions)
			if err != nil {
				return usageErr(err)
			}
			state := systemone.JSON(stateText)
			if cmd.Flags().Changed("state") {
				data, err := readDecisionInput(cmd, statePath)
				if err != nil {
					return usageErr(err)
				}
				state = systemone.State(data)
			}
			c, model, err := o.client(flags)
			if err != nil {
				return err
			}
			req := systemone.Request{Model: model, State: state, Questions: questions}
			if err := req.Validate(); err != nil {
				return usageErr(err)
			}
			if dryRunOK(flags) {
				return decisionPreview(cmd, flags, c, req, nil)
			}
			ctx, cancel := boundCtx(cmd.Context(), flags)
			defer cancel()
			res, err := c.SystemOne(ctx, req)
			if err != nil {
				return decisionAPIError(err)
			}
			if flags.asJSON {
				err = printDecisionJSON(cmd, flags, res)
			} else {
				err = printDecisions(cmd, flags, res)
			}
			if err != nil {
				return err
			}
			for _, g := range gates {
				if g.matches(res.Answers[g.id]) {
					return &cliError{code: decisionGateExitCode, err: fmt.Errorf("decision gate matched: %s\nhint: inspect the answers before continuing", g.raw)}
				}
			}
			return nil
		},
	}
	o.addFlags(cmd)
	cmd.Flags().StringVar(&statePath, "state", "", "State file path or - for stdin; JSON objects/arrays stay structured")
	cmd.Flags().StringVar(&stateText, "state-text", "", "Literal state text")
	cmd.Flags().StringArrayVar(&nouls, "noul", nil, "Repeatable id=instructions binary question")
	cmd.Flags().StringVar(&inlineQuestions, "questions-json", "", "Inline systemone questions map JSON (for agents/MCP)")
	cmd.Flags().StringArrayVar(&failIf, "fail-if", nil, "Exit 9 if any condition matches: id>n, id>=n, id<n, id<=n, id==choice")
	cmd.AddCommand(newDecideBatchCmd(flags))
	return cmd
}

func decisionPreview(cmd *cobra.Command, flags *rootFlags, c *client.Client, req systemone.Request, rows *int) error {
	target, _ := client.SystemOneURL(c.BaseURL)
	preview := map[string]any{"dry_run": true, "method": "POST", "url": target, "body": req}
	if rows != nil {
		preview["rows"] = *rows
	}
	return printDecisionJSON(cmd, flags, preview)
}

// Decisions and previews must retain their wire values. The general compact
// projection decodes through float64, which rounds large JSON integers.
func printDecisionJSON(cmd *cobra.Command, flags *rootFlags, value any) error {
	outputFlags := *flags
	outputFlags.compact = false
	outputFlags.asJSON = true
	return outputFlags.printJSON(cmd, value)
}

// Keep systemone's status handling typed and avoid writing an error envelope to
// stdout, where a batch's JSONL rows (or a successful gate response) belong.
func decisionAPIError(err error) error {
	var httpErr *client.APIError
	if errors.As(err, &httpErr) {
		switch httpErr.StatusCode {
		case 401:
			return authErr(fmt.Errorf("%w\nhint: configure MIXLAYER_API_KEY in the environment; never pass a key on argv", err))
		case 403:
			return authErr(fmt.Errorf("%w\nhint: check classification model and region access for your credentials", err))
		case 402:
			return billingErr(fmt.Errorf("%w\nhint: add credits at https://console.mixlayer.com/app/billing; do not retry until resolved", err))
		case 422:
			return usageErr(fmt.Errorf("%w\nhint: check the model, state, questions and size limits; do not retry unchanged", err))
		case 429:
			return rateLimitErr(fmt.Errorf("%w\nhint: rate limited; honor Retry-After %q before retrying", err, httpErr.RetryAfter))
		case 529:
			return apiErr(fmt.Errorf("%w\nhint: classification capacity unavailable; retry later and honor Retry-After %q", err, httpErr.RetryAfter))
		}
	}
	if errors.Is(err, client.ErrPlaceholderCredential) {
		return authErr(err)
	}
	return apiErr(fmt.Errorf("%w\nhint: check classification service health and the response contract before retrying", err))
}

func printDecisions(cmd *cobra.Command, flags *rootFlags, res *systemone.Response) error {
	ids := make([]string, 0, len(res.Answers))
	for id := range res.Answers {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	var rows [][]string
	for _, id := range ids {
		a := res.Answers[id]
		value, confidence := "", "—"
		if a.Noul != nil {
			value = strconv.FormatFloat(*a.Noul, 'g', -1, 64)
		}
		if a.Score != nil {
			value = strconv.FormatFloat(*a.Score, 'g', -1, 64)
		}
		if a.Choice != nil {
			value = *a.Choice
		}
		if a.Confidence != nil {
			confidence = strconv.FormatFloat(*a.Confidence, 'g', -1, 64)
		}
		rows = append(rows, []string{id, a.Type, value, confidence})
	}
	return flags.printTable(cmd, []string{"ID", "TYPE", "ANSWER", "CONFIDENCE"}, rows)
}

type decisionGate struct {
	id, op, choice, raw string
	number              float64
}

var decisionGatePattern = regexp.MustCompile(`^(.+?)(>=|<=|==|>|<)(.*)$`)

func parseDecisionGates(specs []string, questions systemone.Questions) ([]decisionGate, error) {
	var gates []decisionGate
	for _, spec := range specs {
		m := decisionGatePattern.FindStringSubmatch(spec)
		if m == nil {
			return nil, fmt.Errorf("invalid --fail-if %q; use id>n, id>=n, id<n, id<=n or id==choice", spec)
		}
		g := decisionGate{id: strings.TrimSpace(m[1]), op: m[2], raw: spec, choice: m[3]}
		q, ok := questions[g.id]
		if !ok {
			return nil, fmt.Errorf("--fail-if references unknown question %q", g.id)
		}
		if g.op == "==" {
			if q.Type != "choice" {
				return nil, fmt.Errorf("== requires a choice question")
			}
		} else {
			if q.Type != "noul" && q.Type != "score" {
				return nil, fmt.Errorf("numeric comparisons require noul or score")
			}
			n, err := strconv.ParseFloat(strings.TrimSpace(m[3]), 64)
			if err != nil || math.IsNaN(n) || math.IsInf(n, 0) {
				return nil, fmt.Errorf("--fail-if requires a finite number")
			}
			g.number = n
		}
		gates = append(gates, g)
	}
	return gates, nil
}
func (g decisionGate) matches(a systemone.Answer) bool {
	if g.op == "==" {
		return a.Choice != nil && *a.Choice == g.choice
	}
	n := a.Noul
	if a.Type == "score" {
		n = a.Score
	}
	if n == nil {
		return false
	}
	switch g.op {
	case ">":
		return *n > g.number
	case ">=":
		return *n >= g.number
	case "<":
		return *n < g.number
	case "<=":
		return *n <= g.number
	}
	return false
}
