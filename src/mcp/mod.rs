// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use rusqlite::types::ValueRef;
use serde_json::{Map, Value, json};

use crate::api::{client::Client, config};
use crate::paths::{self, Kind};
use crate::spec_types::{Cmd, Kind as FlagKind};

#[derive(Clone)]
pub struct MixlayerServer {
    tools: Vec<Tool>,
    paths: BTreeMap<String, Vec<String>>,
}

impl Default for MixlayerServer {
    fn default() -> Self {
        Self::new()
    }
}

impl MixlayerServer {
    pub fn new() -> Self {
        // Typed and core workflow schemas are fixed by the Go MCP contract.
        // Command mirror tools below are generated from the Rust command table.
        let mut tools: Vec<Tool> =
            serde_json::from_str(include_str!("static_tools.json")).expect("checked-in typed MCP tools are valid");
        let mut paths = BTreeMap::new();
        fn visit(
            cmds: &[Cmd],
            prefix: &[String],
            inherited_hidden: bool,
            tools: &mut Vec<Tool>,
            paths: &mut BTreeMap<String, Vec<String>>,
        ) {
            for cmd in cmds {
                let mut path = prefix.to_vec();
                path.push(cmd.name.into());
                let hidden = inherited_hidden || cmd.hidden || annotation(cmd, "mcp:hidden") == Some("true");
                if hidden {
                    continue;
                }
                let framework = prefix.is_empty()
                    && matches!(
                        cmd.name,
                        "agent-context" | "auth" | "completion" | "doctor" | "profile" | "sql" | "version" | "which"
                    );
                if framework {
                    continue;
                }
                let endpoint = annotation(cmd, "cli:endpoint").is_some();
                if !endpoint && !cmd.experimental {
                    let name = path.join("_").replace('-', "_");
                    if !tools.iter().any(|t| t.name == name) {
                        tools.push(mirror_tool(&name, cmd));
                        paths.insert(name, path.clone());
                    }
                }
                visit(cmd.subcommands, &path, hidden, tools, paths);
            }
        }
        visit(crate::spec::COMMANDS, &[], false, &mut tools, &mut paths);
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Self { tools, paths }
    }

    fn call(&self, name: &str, args: &Map<String, Value>) -> anyhow::Result<String> {
        match name {
            "context" => context(),
            "sql" => sql(args),
            "chat_create_completion" => chat(args),
            "models_get" => models_get(args),
            "models_list" => models_list(args),
            "ask" | "ladder" | "escalate" | "shield_ask" | "shield_redact" | "doctor" => workflow(name, args),
            _ => mirror(
                self.paths
                    .get(name)
                    .ok_or_else(|| anyhow::anyhow!("unknown workflow"))?,
                args,
            ),
        }
    }
}

fn annotation<'a>(cmd: &'a Cmd, key: &str) -> Option<&'a str> {
    cmd.annotations.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn mirror_tool(name: &str, cmd: &Cmd) -> Tool {
    let mut properties = Map::new();
    for (flag, kind, usage, default) in crate::cli::GLOBAL_FLAGS {
        properties.insert((*flag).into(), flag_schema(flag, *kind, usage, *default));
    }
    for flag in cmd.flags {
        properties.insert(
            flag.name.into(),
            flag_schema(flag.name, flag.kind, flag.usage, flag.default),
        );
    }
    if !cmd.positionals().is_empty() {
        properties.insert("args".into(), json!({"description":"Additional positional arguments or raw CLI flags to append to the command.","type":"string"}));
    }
    let schema = json!({"properties": properties, "required": [], "type": "object"})
        .as_object()
        .cloned()
        .unwrap();
    let read_only = annotation(cmd, "mcp:read-only") == Some("true");
    Tool::new(name.to_owned(), cmd.short.to_owned(), schema).with_annotations(
        ToolAnnotations::new()
            .read_only(read_only)
            .destructive(!read_only)
            .idempotent(false)
            .open_world(true),
    )
}

fn flag_schema(name: &str, kind: FlagKind, usage: &str, default: Option<&str>) -> Value {
    let mut description = usage.trim().to_owned();
    if description.is_empty() {
        description = format!("Value for --{name}");
    }
    let shown_default = if kind == FlagKind::Bool {
        Some(default.unwrap_or("false"))
    } else {
        default
    };
    if let Some(d) = shown_default.filter(|v| !v.is_empty() && *v != "[]") {
        description.push_str(&format!(" (default: {d})"));
    }
    let value_type = match kind {
        FlagKind::Bool => "boolean",
        FlagKind::Int | FlagKind::Int64 | FlagKind::Float => "number",
        _ => "string",
    };
    json!({"description": description, "type": value_type})
}

impl ServerHandler for MixlayerServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("Mixlayer", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|t| t.name == name).cloned()
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if !self.tools.iter().any(|t| t.name == request.name) {
            return Err(McpError::invalid_params("unknown tool", None));
        }
        let args = request.arguments.unwrap_or_default();
        // Tools use the blocking HTTP client and spawn the CLI, so run them
        // on a blocking thread rather than inside the async runtime.
        let server = self.clone();
        let name = request.name.to_string();
        let result = tokio::task::spawn_blocking(move || server.call(&name, &args))
            .await
            .map_err(|e| McpError::internal_error(format!("tool task failed: {e}"), None))?;
        Ok(match result {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]).into(),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]).into(),
        })
    }
}

fn required<'a>(args: &'a Map<String, Value>, key: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{key} is required"))
}

fn client() -> anyhow::Result<Client> {
    Client::new(config::load(None)?, Duration::from_secs(60), 2.0, true)
}

fn chat(args: &Map<String, Value>) -> anyhow::Result<String> {
    required(args, "model")?;
    if !args.get("messages").is_some_and(Value::is_array) {
        anyhow::bail!("messages is required");
    }
    let allowed = [
        "model",
        "messages",
        "tools",
        "tool_choice",
        "temperature",
        "top_p",
        "max_tokens",
        "stream",
    ];
    let mut body = Map::new();
    for key in allowed {
        if let Some(v) = args.get(key) {
            body.insert(key.into(), v.clone());
        }
    }
    Ok(bounded_json(
        "POST",
        client()?.post("/chat/completions", &Value::Object(body))?,
    ))
}

fn models_list(args: &Map<String, Value>) -> anyhow::Result<String> {
    if args.get("dry_run") == Some(&Value::Bool(true)) {
        return Ok(json!({"dry_run":true,"method":"GET","endpoint":"/models"}).to_string());
    }
    Ok(bounded_json("GET", client()?.get("/models")?))
}

fn models_get(args: &Map<String, Value>) -> anyhow::Result<String> {
    let id = required(args, "id")?;
    let data = client()?.get("/models")?;
    fn catalog_items(value: &Value) -> Option<&Vec<Value>> {
        if let Some(array) = value.as_array() {
            return Some(array);
        }
        for key in ["data", "models", "results", "organizations"] {
            if let Some(found) = value.get(key).and_then(catalog_items) {
                return Some(found);
            }
        }
        None
    }
    let items = catalog_items(&data);
    let model = items
        .and_then(|array| array.iter().find(|v| v.get("id").and_then(Value::as_str) == Some(id)))
        .ok_or_else(|| anyhow::anyhow!("model {id:?} not found"))?;
    Ok(bounded_json("GET", model.clone()))
}

const RESULT_MAX_BYTES: usize = 60_000;
const RESULT_MAX_ITEMS: usize = 50;
const BOUND_NOTE: &str = "Typed MCP endpoint response exceeded the tool result budget. Narrow the request with limit, offset, filters, search/sql, or a command-mirror tool with --agent/--compact/--select.";

fn bounded_json(method: &str, value: Value) -> String {
    let raw = value.to_string();
    if method.eq_ignore_ascii_case("GET")
        && let Some(items) = value.as_array()
    {
        for count in (0..=items.len().min(RESULT_MAX_ITEMS)).rev() {
            let mut envelope = json!({"count":items.len(),"items":&items[..count]});
            if count < items.len() {
                envelope["truncated"] = true.into();
                envelope["returned_count"] = count.into();
                envelope["original_bytes"] = raw.len().into();
                envelope["max_bytes"] = RESULT_MAX_BYTES.into();
                envelope["note"] = BOUND_NOTE.into();
            }
            let text = envelope.to_string();
            if text.len() <= RESULT_MAX_BYTES {
                return text;
            }
        }
    }
    if raw.len() <= RESULT_MAX_BYTES {
        return raw;
    }
    if method.eq_ignore_ascii_case("GET")
        && let Some(object) = value.as_object()
    {
        let arrays: Vec<_> = object
            .iter()
            .filter_map(|(key, value)| value.as_array().map(|rows| (key, rows)))
            .collect();
        if let [(key, rows)] = arrays.as_slice() {
            for count in (0..=rows.len().min(RESULT_MAX_ITEMS)).rev() {
                let mut envelope = value.clone();
                envelope[*key] = Value::Array(rows[..count].to_vec());
                if count < rows.len() {
                    envelope["_pp_truncated"] = true.into();
                    envelope["_pp_total_count"] = rows.len().into();
                    envelope["_pp_returned_count"] = count.into();
                    envelope["_pp_original_bytes"] = raw.len().into();
                    envelope["_pp_max_bytes"] = RESULT_MAX_BYTES.into();
                    envelope["_pp_note"] = BOUND_NOTE.into();
                }
                let text = envelope.to_string();
                if text.len() <= RESULT_MAX_BYTES {
                    return text;
                }
            }
        }
    }
    let mut end = raw.len().min(RESULT_MAX_BYTES / 2);
    loop {
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        let envelope = json!({
            "truncated":true, "original_bytes":raw.len(), "max_bytes":RESULT_MAX_BYTES,
            "preview":&raw[..end], "note":"Typed MCP endpoint response exceeded the tool result budget and was not a recognized list envelope. Narrow the request with filters, search/sql, or a command-mirror tool with --agent/--compact/--select."
        }).to_string();
        if envelope.len() <= RESULT_MAX_BYTES || end == 0 {
            return envelope;
        }
        end /= 2;
    }
}

fn context() -> anyhow::Result<String> {
    let mut dirs = Map::new();
    for (name, kind) in [
        ("config_dir", Kind::Config),
        ("data_dir", Kind::Data),
        ("state_dir", Kind::State),
        ("cache_dir", Kind::Cache),
    ] {
        if let Ok(path) = paths::dir(kind) {
            dirs.insert(name.into(), Value::String(path.display().to_string()));
        }
    }
    Ok(serde_json::to_string_pretty(&json!({
        "api":"mixlayer",
        "skill":crate::skill::TEXT,
        "description":"Mixlayer CLI: OpenAI-compatible model calls, model comparison, cost ledger and local PII masking",
        "paths":dirs,
        "tool_surface":"Typed tools: chat_create_completion, models_list, models_get. Workflow tools: ask, ladder, escalate, shield_ask, shield_redact, shield_scan, shield_audit, grep, sql, savings, doctor. Workflow tools run the companion mxl binary.",
        "auth":{"type":"bearer_token","env_vars":[{"name":"MIXLAYER_API_KEY","kind":"per_call","required":true,"sensitive":true,"description":"Mixlayer API key. The MCP server reads it from its own environment or the CLI credentials file; never pass it as a tool argument."}]},
        "query_tips":[
            "Call models_list first and pick model IDs from it; IDs and prices change.",
            "Use dry_run on ask, ladder, escalate and shield_ask to preview the exact request before spending credits or sending data.",
            "Send personal data only through shield_ask, which masks it locally and restores it in the answer.",
            "Use escalate to start on a cheap model and climb only when the answer is unsure; use ladder to compare models side by side.",
            "Use sql or grep on the local ledger to find earlier answers instead of asking again."
        ]
    }))?)
}

fn sql(args: &Map<String, Value>) -> anyhow::Result<String> {
    let query = required(args, "query")?;
    // Guard before opening the database so the refusal also holds when no
    // ledger exists. The store applies its own read-only and privacy checks.
    let private = regex::Regex::new(
        r"(?i)(^|[^a-z0-9_])(vault|shield_dictionary|sqlite_master|sqlite_schema|pragma_[a-z_]+)([^a-z0-9_]|$)",
    )?;
    if private.is_match(query) {
        anyhow::bail!(
            "the vault and shield dictionary hold private values and cannot be queried over MCP; use the mixlayer CLI locally"
        );
    }
    validate_read_query(query)?;
    let db = crate::store::Store::open_read_only(paths::dir(Kind::Data)?.join("data.db"))?;
    // The authorizer is the real boundary: it refuses private tables even
    // when a query reaches them through a view or trigger.
    let _guard = crate::store::PrivateReadGuard::install(db.connection());
    let mut stmt = db.connection().prepare(query).map_err(|e| {
        if e.to_string().contains("not authorized") || e.to_string().contains("prohibited") {
            anyhow::anyhow!(
                "the vault and shield dictionary hold private values and cannot be queried over MCP; use the mixlayer CLI locally"
            )
        } else {
            e.into()
        }
    })?;
    if !stmt.readonly() {
        anyhow::bail!("only SELECT queries are allowed");
    }
    let columns: Vec<String> = stmt.column_names().iter().map(|s| (*s).to_owned()).collect();
    let mut rows = stmt.query([])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let mut object = Map::new();
        for (i, name) in columns.iter().enumerate() {
            let value = match row.get_ref(i)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => Value::from(n),
                ValueRef::Real(n) => Value::from(n),
                ValueRef::Text(s) | ValueRef::Blob(s) => Value::String(String::from_utf8_lossy(s).into_owned()),
            };
            object.insert(name.clone(), value);
        }
        result.push(object);
    }
    Ok(serde_json::to_string_pretty(&result)?)
}

fn validate_read_query(query: &str) -> anyhow::Result<()> {
    let stripped = strip_leading_sql_noise(query);
    if has_trailing_sql_statement(stripped) {
        anyhow::bail!("only a single SELECT or WITH statement is allowed");
    }
    let upper = stripped.to_ascii_uppercase();
    if !upper.starts_with("SELECT") && !upper.starts_with("WITH") {
        anyhow::bail!("only SELECT queries are allowed");
    }
    Ok(())
}

fn strip_leading_sql_noise(mut query: &str) -> &str {
    loop {
        query = query.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ';');
        if let Some(rest) = query.strip_prefix("--") {
            query = match rest.find('\n') {
                Some(i) => &rest[i + 1..],
                None => return "",
            };
        } else if let Some(rest) = query.strip_prefix("/*") {
            query = match rest.find("*/") {
                Some(i) => &rest[i + 2..],
                None => return "",
            };
        } else {
            return query;
        }
    }
}

fn has_trailing_sql_statement(query: &str) -> bool {
    let bytes = query.as_bytes();
    let (mut single, mut double, mut backtick, mut bracket, mut line, mut block) =
        (false, false, false, false, false, false);
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        let next = bytes.get(i + 1).copied().unwrap_or(0);
        if line {
            if ch == b'\n' {
                line = false;
            }
            i += 1;
            continue;
        }
        if block {
            if ch == b'*' && next == b'/' {
                block = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if single {
            if ch == b'\'' {
                if next == b'\'' {
                    i += 2;
                    continue;
                }
                single = false;
            }
            i += 1;
            continue;
        }
        if double {
            if ch == b'"' {
                if next == b'"' {
                    i += 2;
                    continue;
                }
                double = false;
            }
            i += 1;
            continue;
        }
        if backtick {
            if ch == b'`' {
                if next == b'`' {
                    i += 2;
                    continue;
                }
                backtick = false;
            }
            i += 1;
            continue;
        }
        if bracket {
            if ch == b']' {
                bracket = false;
            }
            i += 1;
            continue;
        }
        match (ch, next) {
            (b'-', b'-') => {
                line = true;
                i += 2;
            }
            (b'/', b'*') => {
                block = true;
                i += 2;
            }
            (b'\'', _) => {
                single = true;
                i += 1;
            }
            (b'"', _) => {
                double = true;
                i += 1;
            }
            (b'`', _) => {
                backtick = true;
                i += 1;
            }
            (b'[', _) => {
                bracket = true;
                i += 1;
            }
            (b';', _) => {
                if !strip_leading_sql_noise(&query[i + 1..]).is_empty() {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

fn cli_path() -> anyhow::Result<PathBuf> {
    let name = if cfg!(windows) { "mxl.exe" } else { "mxl" };
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name(name);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    if let Some(path) = std::env::var_os("MIXLAYER_CLI_PATH").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    anyhow::bail!("companion mxl CLI not found: tried sibling lookup, MIXLAYER_CLI_PATH, and PATH")
}

fn run_cli(args: &[String]) -> anyhow::Result<String> {
    let bin = cli_path()?;
    let output = Command::new(&bin).args(args).output()?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let msg = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    anyhow::bail!("cli {}: {} ({msg})", bin.display(), output.status)
}

fn workflow(name: &str, args: &Map<String, Value>) -> anyhow::Result<String> {
    let mut cli = vec!["--agent".to_owned()];
    let positional = match name {
        "ask" | "ladder" | "escalate" | "shield_ask" => Some(required(args, "question")?.to_owned()),
        "shield_redact" => Some(required(args, "file")?.to_owned()),
        _ => None,
    };
    if name == "shield_ask" {
        required(args, "upstream")?;
    }
    if name == "shield_redact" && args.get("dry_run") == Some(&Value::Bool(true)) {
        return Ok(json!({"dry_run":true,"operation":"shield redact","file":positional,"detector":args.get("detector").and_then(Value::as_str).unwrap_or("regex"),"network_calls":0,"vault_writes":0}).to_string());
    }
    if args.get("dry_run") == Some(&Value::Bool(true)) {
        cli.push("--dry-run".into());
    }
    match name {
        "shield_ask" => cli.extend(["shield".into(), "ask".into()]),
        "shield_redact" => cli.extend(["shield".into(), "redact".into()]),
        _ => cli.push(name.into()),
    }
    for key in [
        "model",
        "rungs",
        "detector",
        "data",
        "upstream",
        "upstream_base_url",
        "upstream_key_env",
    ] {
        if let Some(value) = args.get(key) {
            let text = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("{key} must be a string"))?;
            if !text.trim().is_empty() {
                cli.extend([format!("--{}", key.replace('_', "-")), text.into()]);
            }
        }
    }
    if let Some(value) = args.get("confidence") {
        let n = value
            .as_f64()
            .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
            .ok_or_else(|| anyhow::anyhow!("confidence must be a number from 0 to 1"))?;
        cli.extend(["--confidence".into(), n.to_string()]);
    }
    if args.get("reasoning") == Some(&Value::Bool(true)) {
        cli.push("--reasoning".into());
    }
    if let Some(word) = positional {
        cli.extend(["--".into(), word]);
    }
    run_cli(&cli)
}

fn mirror(path: &[String], args: &Map<String, Value>) -> anyhow::Result<String> {
    run_cli(&mirror_cli_args(path, args)?)
}

fn mirror_cli_args(path: &[String], args: &Map<String, Value>) -> anyhow::Result<Vec<String>> {
    let mut cli = path.to_vec();
    let blocked: BTreeSet<&str> = [
        "args", "base-url", "client", "config", "deliver", "home", "profile", "token",
    ]
    .into();
    let mut keys: Vec<&String> = args
        .keys()
        .filter(|k| !blocked.contains(k.as_str()) && !k.contains('='))
        .collect();
    keys.sort();
    for key in keys {
        let value = &args[key];
        match value {
            Value::Bool(true) => cli.push(format!("--{key}")),
            Value::Bool(false) | Value::Null => {}
            Value::String(s) if !s.is_empty() => cli.extend([format!("--{key}"), s.clone()]),
            Value::Number(n) => cli.extend([format!("--{key}"), n.to_string()]),
            Value::Array(a) if !a.is_empty() => cli.extend([
                format!("--{key}"),
                a.iter().map(value_text).collect::<Vec<_>>().join(","),
            ]),
            other => cli.extend([format!("--{key}"), value_text(other)]),
        }
    }
    if let Some(raw) = args.get("args").and_then(Value::as_str) {
        let words = split_shell_args(raw);
        if let Some(bad) = words.iter().find(|s| s.starts_with('-') && *s != "-") {
            anyhow::bail!(
                "flag-like argument {bad:?} not allowed in positional args field; use structured tool parameters instead"
            );
        }
        // `--` makes every word a plain argument, so `args` can never name a
        // subcommand (e.g. reach the hidden `shield dict` from the `shield`
        // tool).
        if !words.is_empty() {
            cli.push("--".into());
            cli.extend(words);
        }
    }
    Ok(cli)
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

fn split_shell_args(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut single = false;
    let mut double = false;
    let mut escape = false;
    for ch in input.chars() {
        if escape {
            word.push(ch);
            escape = false;
        } else if ch == '\\' && !single {
            escape = true;
        } else if ch == '\'' && !double {
            single = !single;
        } else if ch == '"' && !single {
            double = !double;
        } else if (ch == ' ' || ch == '\t') && !single && !double {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(ch);
        }
    }
    if escape {
        word.push('\\');
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mirror_surface_has_no_private_commands() {
        let server = MixlayerServer::new();
        assert_eq!(server.tools.len(), 21);
        for name in [
            "vault",
            "shield_dict",
            "agent_setup",
            "agent_skill_print",
            "keys",
            "budget",
            "audit",
        ] {
            assert!(!server.tools.iter().any(|t| t.name == name));
        }
        for name in ["whoami", "spend"] {
            assert!(server.tools.iter().any(|t| t.name == name));
        }
    }

    #[test]
    fn go_sql_allowlist_and_bypass_cases() {
        for query in [
            "SELECT 1",
            "with r as (select 1) select * from r",
            ";SELECT 1",
            "-- comment\nSELECT 1",
            "/* comment */ SELECT 1",
            "SELECT 1; -- trailing",
            "SELECT 'a;b'",
            "SELECT * FROM [semi;colon]",
        ] {
            assert!(validate_read_query(query).is_ok(), "{query:?}");
        }
        for query in [
            "VACUUM INTO '/tmp/x.db'",
            "ATTACH DATABASE '/tmp/x.db' AS evil",
            "/* x */ VACUUM INTO '/tmp/x.db'",
            "-- x\nVACUUM INTO '/tmp/x.db'",
            "SELECT 1; ATTACH DATABASE '/tmp/x.db' AS evil",
            "SELECT 1; SELECT 2",
            "",
        ] {
            assert!(validate_read_query(query).is_err(), "{query:?}");
        }
    }

    #[test]
    fn sql_private_tables_are_rejected_before_open() {
        for query in [
            "SELECT value FROM vault",
            "select * from \"vault\"",
            "SELECT term FROM shield_dictionary",
            "WITH v AS (SELECT * FROM [vault]) SELECT * FROM v",
            "SELECT sql FROM sqlite_master",
            "SELECT * FROM pragma_table_info('vault')",
        ] {
            let mut args = Map::new();
            args.insert("query".into(), Value::String(query.into()));
            assert!(
                sql(&args).unwrap_err().to_string().contains("private values"),
                "{query:?}"
            );
        }
    }

    #[test]
    fn shell_split_matches_go_cases() {
        for (raw, expected) in [
            ("", vec![]),
            ("contacts", vec!["contacts"]),
            ("inbox health", vec!["inbox", "health"]),
            ("  foo   bar  ", vec!["foo", "bar"]),
            ("foo\tbar", vec!["foo", "bar"]),
            ("\"hello world\"", vec!["hello world"]),
            ("contacts \"john doe\" active", vec!["contacts", "john doe", "active"]),
            ("'a b' c", vec!["a b", "c"]),
            ("a\\ b c", vec!["a b", "c"]),
        ] {
            assert_eq!(split_shell_args(raw), expected, "{raw:?}");
        }
    }

    #[test]
    fn mirror_args_block_root_controls_and_raw_flags() {
        let mut args = Map::new();
        for key in [
            "base-url", "client", "config", "deliver", "home", "profile", "token", "limit=99",
        ] {
            args.insert(key.into(), Value::String("untrusted".into()));
        }
        args.insert("limit".into(), json!(10));
        args.insert("args".into(), Value::String("contacts".into()));
        assert_eq!(
            mirror_cli_args(&["grep".into()], &args).unwrap(),
            ["grep", "--limit", "10", "--", "contacts"]
        );
        args.insert("args".into(), Value::String("contacts --config /tmp/evil".into()));
        assert!(mirror_cli_args(&["grep".into()], &args).is_err());
    }

    #[test]
    fn context_includes_embedded_privacy_rule() {
        let value: Value = serde_json::from_str(&context().unwrap()).unwrap();
        assert!(value["skill"].as_str().unwrap().contains("Never print vault values"));
    }

    #[test]
    fn typed_results_are_bounded_like_go() {
        let items: Vec<Value> = (0..75).map(|_| Value::String("x".repeat(1600))).collect();
        let original = Value::Array(items.clone()).to_string();
        let text = bounded_json("GET", Value::Array(items));
        let result: Value = serde_json::from_str(&text).unwrap();
        assert!(text.len() <= RESULT_MAX_BYTES);
        assert_eq!(result["count"], 75);
        assert_eq!(result["truncated"], true);
        assert_eq!(
            result["returned_count"].as_u64().unwrap(),
            result["items"].as_array().unwrap().len() as u64
        );
        assert_eq!(result["original_bytes"], original.len());

        let groups: Vec<Value> = (0..75)
            .map(|_| json!({"id":"gggggggg","name":"verbose group name ".repeat(90)}))
            .collect();
        let value = json!({"groups":groups,"cursor":"next-page"});
        let text = bounded_json("GET", value);
        let result: Value = serde_json::from_str(&text).unwrap();
        assert!(text.len() <= RESULT_MAX_BYTES);
        assert_eq!(result["cursor"], "next-page");
        assert_eq!(result["_pp_truncated"], true);
        assert_eq!(result["_pp_total_count"], 75);

        for method in ["GET", "POST"] {
            let text = bounded_json(method, json!({"blob":"z".repeat(RESULT_MAX_BYTES + 10_000)}));
            let result: Value = serde_json::from_str(&text).unwrap();
            assert!(text.len() <= RESULT_MAX_BYTES);
            assert_eq!(result["truncated"], true);
            assert!(result["preview"].as_str().is_some_and(|s| !s.is_empty()));
        }
    }

    #[test]
    fn args_never_select_a_subcommand() {
        let path = vec!["shield".to_string()];
        let mut args = Map::new();
        args.insert("args".into(), Value::String("dict list".into()));
        let cli = mirror_cli_args(&path, &args).unwrap();
        assert_eq!(cli, ["shield", "--", "dict", "list"]);
        args.insert("args".into(), Value::String("restructure /etc/passwd".into()));
        assert_eq!(mirror_cli_args(&path, &args).unwrap()[1], "--");
    }
}
