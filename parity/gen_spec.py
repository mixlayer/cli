#!/usr/bin/env python3
"""One-time generator: turns the Go CLI's agent-context snapshot into
src/spec.rs. After generation, src/spec.rs is the source of truth and is
edited by hand; this script stays only to document where the table came from.

Usage: python3 parity/gen_spec.py > src/spec.rs
"""
import json
import pathlib

HERE = pathlib.Path(__file__).parent
SNAP = json.loads((HERE / "go-agent-context-jini.json").read_text())

KINDS = {
    "string": "Str", "bool": "Bool", "int": "Int", "int64": "Int64",
    "float64": "Float", "duration": "Duration", "stringSlice": "StrSlice",
    "stringArray": "StrArray",
}
SHORT = {("shield redact", "output"): "o", ("shield ingest", "output"): "o",
         ("shield restructure", "output"): "o"}
EXPERIMENTAL = {"decide"}
HIDDEN = {"agent skill-ref"}
# cobra builds these itself; clap builds `help`, and we build completion with clap_complete.
GENERATED_BY_FRAMEWORK = {"help"}


def lit(s):
    return json.dumps(s, ensure_ascii=False)


def emit(cmd, path, depth):
    ind = "    " * depth
    full = (path + " " + cmd["name"]).strip()
    flags = []
    for f in cmd.get("flags", []):
        short = SHORT.get((full, f["name"]))
        flags.append(
            f"{ind}        Flag {{ name: {lit(f['name'])}, short: {repr(short) if short else 'None'}"
            .replace("'", "'") +
            f", kind: Kind::{KINDS[f['type']]}, usage: {lit(f.get('usage', ''))}, default: {'Some(' + lit(f['default']) + ')' if 'default' in f else 'None'} }},"
        )
    flags = [l.replace("short: '", "short: Some('") .replace("', kind", "'), kind") for l in flags]
    ann = ", ".join(f"({lit(k)}, {lit(v)})" for k, v in sorted((cmd.get("annotations") or {}).items()))
    subs = [s for s in cmd.get("subcommands", []) if s["name"] not in GENERATED_BY_FRAMEWORK]
    out = [f"{ind}Cmd {{",
           f"{ind}    name: {lit(cmd['name'])},",
           f"{ind}    usage: {lit(cmd.get('use', cmd['name']))},",
           f"{ind}    short: {lit(cmd.get('short', ''))},",
           f"{ind}    annotations: &[{ann}],",
           f"{ind}    experimental: {'true' if cmd['name'] in EXPERIMENTAL and depth == 1 else 'false'},",
           f"{ind}    hidden: {'true' if full in HIDDEN else 'false'},",
           f"{ind}    flags: &["] + flags + [f"{ind}    ],", f"{ind}    subcommands: &["]
    for s in subs:
        out += emit(s, full, depth + 2)
    out += [f"{ind}    ],", f"{ind}}},"]
    return out


lines = [
    "// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.",
    "",
    "//! The command table. Every command, flag and annotation the CLI exposes is",
    "//! declared here once; clap, `agent-context`, `which` and the MCP server all",
    "//! read it. Generated from the Go CLI's agent-context by parity/gen_spec.py,",
    "//! then maintained by hand.",
    "",
    "use crate::spec_types::{Cmd, Flag, Kind};",
    "",
    "pub static COMMANDS: &[Cmd] = &[",
]
for c in SNAP["commands"]:
    if c["name"] in GENERATED_BY_FRAMEWORK:
        continue
    lines += emit(c, "", 1)
lines.append("];")
print("\n".join(lines))
