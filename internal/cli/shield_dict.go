// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

package cli

import (
	"fmt"
	"strings"

	"github.com/mixlayer/cli/internal/shield"
	"github.com/spf13/cobra"
)

func newShieldDictCmd(flags *rootFlags) *cobra.Command {
	var dbPath string
	cmd := &cobra.Command{Use: "dict", Short: "Manage local terms that every shield detector masks", RunE: parentNoSubcommandRunE(flags), Annotations: map[string]string{"mcp:hidden": "true"}}
	cmd.PersistentFlags().StringVar(&dbPath, "db", "", "SQLite database file path")
	var kind string
	add := &cobra.Command{Use: "add <term>", Short: "Add or update a local dictionary term", Example: `  mixlayer shield dict add "Zora Quill" --kind PERSON`, Args: cobra.ExactArgs(1), RunE: func(cmd *cobra.Command, args []string) error {
		kind = strings.ToUpper(strings.TrimSpace(kind))
		if !shield.ValidKind(kind) || strings.TrimSpace(args[0]) == "" {
			return usageErr(fmt.Errorf("a nonempty term and valid --kind are required"))
		}
		s, err := openMixStore(cmd.Context(), dbPath)
		if err != nil {
			return err
		}
		defer s.Close()
		if err := s.AddDictionaryTerm(cmd.Context(), args[0], kind); err != nil {
			return err
		}
		return flags.printJSON(cmd, map[string]string{"term": strings.TrimSpace(args[0]), "kind": kind})
	}}
	add.Flags().StringVar(&kind, "kind", "", "Entity kind for the term")
	list := &cobra.Command{Use: "list", Short: "List local dictionary terms", Example: "  mixlayer shield dict list --agent", RunE: func(cmd *cobra.Command, args []string) error {
		s, err := openMixStore(cmd.Context(), dbPath)
		if err != nil {
			return err
		}
		defer s.Close()
		terms, err := s.DictionaryTerms(cmd.Context())
		if err != nil {
			return err
		}
		return flags.printJSON(cmd, terms)
	}}
	remove := &cobra.Command{Use: "remove <term>", Short: "Remove a local dictionary term", Example: `  mixlayer shield dict remove "Zora Quill"`, Args: cobra.ExactArgs(1), RunE: func(cmd *cobra.Command, args []string) error {
		s, err := openMixStore(cmd.Context(), dbPath)
		if err != nil {
			return err
		}
		defer s.Close()
		removed, err := s.RemoveDictionaryTerm(cmd.Context(), args[0])
		if err != nil {
			return err
		}
		return flags.printJSON(cmd, map[string]any{"removed": removed})
	}}
	cmd.AddCommand(add, list, remove)
	return cmd
}
