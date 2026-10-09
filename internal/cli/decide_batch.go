// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.
package cli

import (
	"bufio"
	"context"
	"encoding/csv"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/mixlayer/cli/internal/client"
	"github.com/mixlayer/cli/internal/systemone"
	"github.com/spf13/cobra"
)

type decisionRow struct {
	number int
	state  json.RawMessage
	err    error
}
type decisionRowError struct {
	Message string `json:"message"`
	Code    int    `json:"code"`
}
type decisionRowResult struct {
	Row     int                         `json:"row"`
	Answers map[string]systemone.Answer `json:"answers,omitempty"`
	Error   *decisionRowError           `json:"error,omitempty"`
	Usage   *systemone.Usage            `json:"usage"`
}

func newDecideBatchCmd(flags *rootFlags) *cobra.Command {
	var o decideOptions
	var input, out, stateField string
	var concurrency, maxRows int
	cmd := &cobra.Command{
		Use: "batch", Short: "Evaluate CSV or JSONL rows with shared questions",
		Long:        "Evaluate up to --max-rows rows using shared --questions. Outputs JSONL in completion\norder with one-based row numbers; malformed and rejected rows do not stop later rows.\nAny row failure exits 6 after the summary on stderr. --timeout applies per request.\nJSONL rows must be objects; CSV values are strings. --state-field selects one field.\nDry run prints the first request and row count without opening --out or calling HTTP.",
		Annotations: map[string]string{"mcp:hidden": "true", "cli:no-error-path-probe": "true"},
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) > 0 || input == "" || o.questions == "" {
				return usageErr(fmt.Errorf("decide batch requires --input and --questions"))
			}
			if concurrency < 1 || concurrency > 64 {
				return usageErr(fmt.Errorf("--concurrency must be between 1 and 64"))
			}
			if maxRows < 1 {
				return usageErr(fmt.Errorf("--max-rows must be positive"))
			}
			questions, err := decisionQuestions(cmd, o.questions, "", nil)
			if err != nil {
				return usageErr(err)
			}
			c, model, err := o.client(flags)
			if err != nil {
				return err
			}
			template := systemone.Request{Model: model, State: systemone.JSON("batch validation"), Questions: questions}
			if err := template.Validate(); err != nil {
				return usageErr(err)
			}
			file, err := os.Open(input)
			if err != nil {
				return usageErr(err)
			}
			defer file.Close()
			next, err := decisionRowReader(file, input, stateField)
			if err != nil {
				return usageErr(err)
			}
			if dryRunOK(flags) {
				count := 0
				var first decisionRow
				for count < maxRows {
					row, ok := next()
					if !ok {
						break
					}
					count++
					if count == 1 {
						first = row
					}
				}
				if count == 0 {
					return usageErr(fmt.Errorf("batch input has no rows"))
				}
				if first.err != nil {
					return usageErr(fmt.Errorf("first row: %w", first.err))
				}
				template.State = first.state
				if err := template.Validate(); err != nil {
					return usageErr(err)
				}
				return decisionPreview(cmd, flags, c, template, &count)
			}
			writer := cmd.OutOrStdout()
			if out != "" {
				for _, source := range []string{input, o.questions} {
					if source == "-" {
						continue
					}
					same, err := sameDecisionFile(source, out)
					if err != nil {
						return usageErr(err)
					}
					if same {
						return usageErr(fmt.Errorf("--out must not overwrite an input or questions file"))
					}
				}
				dest, err := os.OpenFile(out, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0600)
				if err != nil {
					return err
				}
				defer dest.Close()
				writer = dest
			}
			return runDecisionBatch(cmd, flags, c, template, next, writer, concurrency, maxRows)
		},
	}
	o.addFlags(cmd)
	cmd.Flags().StringVar(&input, "input", "", "Input rows.jsonl or rows.csv")
	cmd.Flags().StringVar(&out, "out", "", "Write result JSONL to this file instead of stdout")
	cmd.Flags().StringVar(&stateField, "state-field", "", "Use this field as state (default: whole row object)")
	cmd.Flags().IntVar(&concurrency, "concurrency", 4, "Concurrent requests (1–64)")
	cmd.Flags().IntVar(&maxRows, "max-rows", 10000, "Process at most this many rows")
	return cmd
}

func sameDecisionFile(a, b string) (bool, error) {
	ai, err := os.Stat(a)
	if err != nil {
		return false, err
	}
	bi, err := os.Stat(b)
	if os.IsNotExist(err) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	return os.SameFile(ai, bi), nil
}

type nextDecisionRow func() (decisionRow, bool)

func decisionRowReader(r io.Reader, name, field string) (nextDecisionRow, error) {
	var read func() (map[string]json.RawMessage, error)
	switch strings.ToLower(filepath.Ext(name)) {
	case ".jsonl":
		reader := bufio.NewReader(r)
		done := false
		read = func() (map[string]json.RawMessage, error) {
			if done {
				return nil, io.EOF
			}
			var line []byte
			oversized := false
			for {
				chunk, err := reader.ReadSlice('\n')
				if len(line)+len(chunk) > systemone.MaxBody {
					oversized = true
				}
				if !oversized {
					line = append(line, chunk...)
				}
				if errors.Is(err, bufio.ErrBufferFull) {
					continue
				}
				if errors.Is(err, io.EOF) {
					done = true
					if len(line) == 0 && !oversized {
						return nil, io.EOF
					}
				} else if err != nil {
					done = true
					return nil, err
				}
				break
			}
			if oversized {
				return nil, fmt.Errorf("JSONL row exceeds 8 MiB")
			}
			var obj map[string]json.RawMessage
			if err := json.Unmarshal(line, &obj); err != nil || obj == nil {
				return nil, fmt.Errorf("JSONL row must be a JSON object")
			}
			return obj, nil
		}
	case ".csv":
		reader := csv.NewReader(r)
		header, err := reader.Read()
		if err != nil {
			return nil, fmt.Errorf("read CSV header: %w", err)
		}
		seen := map[string]bool{}
		for _, h := range header {
			if strings.TrimSpace(h) == "" || seen[h] {
				return nil, fmt.Errorf("CSV headers must be nonblank and unique")
			}
			seen[h] = true
		}
		read = func() (map[string]json.RawMessage, error) {
			values, err := reader.Read()
			if err != nil {
				return nil, err
			}
			obj := map[string]json.RawMessage{}
			for i, h := range header {
				obj[h] = systemone.JSON(values[i])
			}
			return obj, nil
		}
	default:
		return nil, fmt.Errorf("--input must be a .jsonl or .csv file")
	}
	number := 0
	return func() (decisionRow, bool) {
		obj, err := read()
		if errors.Is(err, io.EOF) {
			return decisionRow{}, false
		}
		number++
		row := decisionRow{number: number, err: err}
		if err != nil {
			return row, true
		}
		if field == "" {
			row.state = systemone.JSON(obj)
		} else {
			v, ok := obj[field]
			if !ok {
				row.err = fmt.Errorf("missing state field %q", field)
			} else {
				row.state = v
			}
		}
		return row, true
	}, nil
}

func runDecisionBatch(cmd *cobra.Command, flags *rootFlags, c *client.Client, template systemone.Request, next nextDecisionRow, writer io.Writer, concurrency, maxRows int) error {
	ctx, cancel := context.WithCancel(cmd.Context())
	defer cancel()
	jobs := make(chan decisionRow, concurrency)
	results := make(chan decisionRowResult, concurrency)
	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		defer close(jobs)
		for i := 0; i < maxRows; i++ {
			if ctx.Err() != nil {
				return
			}
			row, ok := next()
			if !ok {
				return
			}
			select {
			case jobs <- row:
			case <-ctx.Done():
				return
			}
		}
	}()
	for i := 0; i < concurrency; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for row := range jobs {
				if ctx.Err() != nil {
					return
				}
				result := decisionRowResult{Row: row.number}
				req := template
				req.State = row.state
				err := row.err
				if err == nil {
					err = req.Validate()
				}
				if err != nil {
					err = usageErr(err)
				} else {
					rowCtx, rowCancel := boundCtx(ctx, flags)
					response, callErr := c.SystemOne(rowCtx, req)
					rowCancel()
					if callErr != nil {
						err = decisionAPIError(callErr)
					} else {
						result.Answers = response.Answers
						result.Usage = response.Usage
					}
				}
				if err != nil {
					result.Error = &decisionRowError{Message: err.Error(), Code: ExitCode(err)}
				}
				select {
				case results <- result:
				case <-ctx.Done():
					return
				}
			}
		}()
	}
	go func() { wg.Wait(); close(results) }()
	encoder := json.NewEncoder(writer)
	ok, failed := 0, 0
	var inputTokens uint64
	var writeErr error
	for result := range results {
		if writeErr != nil {
			continue
		}
		if err := encoder.Encode(result); err != nil {
			writeErr = err
			cancel()
			continue
		}
		if result.Error != nil {
			failed++
		} else {
			ok++
			inputTokens += result.Usage.InputTokens
		}
	}
	fmt.Fprintf(cmd.ErrOrStderr(), "rows ok=%d failed=%d; total input tokens=%d\n", ok, failed, inputTokens)
	if writeErr != nil {
		return writeErr
	}
	if err := cmd.Context().Err(); err != nil {
		return err
	}
	if failed > 0 {
		return partialFailureErr(fmt.Errorf("%d batch rows failed; inspect row errors", failed))
	}
	if ok == 0 {
		return usageErr(fmt.Errorf("batch input has no rows"))
	}
	return nil
}
