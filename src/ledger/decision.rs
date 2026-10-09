use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::Path;
use std::time::Duration;

use clap::ArgMatches;
use regex::Regex;
use reqwest::blocking::Client;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use crate::Globals;
use crate::api::config;
use crate::exit::{self, Code};

const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_STATE: usize = 4 * 1024 * 1024;
const MAX_QUESTION: usize = 16 * 1024;
fn arg<'a>(m: &'a ArgMatches, k: &str) -> Option<&'a str> {
    m.try_get_one::<String>(k).ok().flatten().map(String::as_str)
}
fn changed(m: &ArgMatches, k: &str) -> bool {
    m.value_source(k) == Some(clap::parser::ValueSource::CommandLine)
}
fn values(m: &ArgMatches, k: &str) -> Vec<String> {
    m.try_get_many::<String>(k)
        .ok()
        .flatten()
        .map(|v| v.cloned().collect())
        .unwrap_or_default()
}
fn usage(e: impl std::fmt::Display) -> anyhow::Error {
    exit::usage(e.to_string())
}
fn print(v: &Value, g: &Globals) -> anyhow::Result<()> {
    let mut flags = g.clone();
    flags.compact = false;
    crate::output::print_json(v, &flags)
}
fn read(path: &str) -> anyhow::Result<Vec<u8>> {
    let mut data = Vec::new();
    if path == "-" {
        io::stdin().take((MAX_BODY + 1) as u64).read_to_end(&mut data)?;
    } else {
        File::open(path)?.take((MAX_BODY + 1) as u64).read_to_end(&mut data)?;
    }
    if data.len() > MAX_BODY {
        anyhow::bail!("input exceeds 8 MiB");
    }
    Ok(data)
}
pub fn model(m: &ArgMatches, name: &str) -> anyhow::Result<String> {
    if let Some(v) = arg(m, name).filter(|v| !v.trim().is_empty()) {
        return Ok(v.into());
    }
    let cfg = config::load(arg(m, "config")).map_err(|e| exit::err(Code::Config, e.to_string()))?;
    let path = cfg.legacy_path.as_ref().unwrap_or(&cfg.path);
    let v = fs::read_to_string(path)
        .ok()
        .and_then(|s| s.parse::<toml::Value>().ok())
        .and_then(|v| {
            v.get("classification_model")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        });
    v.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        usage("classification model is required\nhint: set --model or classification_model in config.toml")
    })
}
fn systemone_url(base: &str) -> anyhow::Result<String> {
    let mut u = url::Url::parse(base)
        .map_err(|_| usage("endpoint must be an HTTP(S) base URL without credentials, query or fragment"))?;
    if !["http", "https"].contains(&u.scheme())
        || u.host().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(usage(
            "endpoint must be an HTTP(S) base URL without credentials, query or fragment",
        ));
    }
    let p = u.path().trim_end_matches('/');
    let p = if p.is_empty() { "/v1".to_owned() } else { p.to_owned() };
    if p.ends_with("/systemone") {
        u.set_path(&p);
    } else {
        u.set_path(&format!("{p}/systemone"));
    }
    Ok(u.into())
}
pub fn target_url(m: &ArgMatches) -> anyhow::Result<String> {
    let cfg = config::load(arg(m, "config")).map_err(|e| exit::err(Code::Config, e.to_string()))?;
    systemone_url(arg(m, "endpoint").filter(|s| !s.is_empty()).unwrap_or(&cfg.base_url))
}
#[derive(Clone)]
struct Transport {
    url: String,
    auth: String,
    headers: Vec<(String, String)>,
    client: Client,
}
impl Transport {
    fn new(m: &ArgMatches) -> anyhow::Result<Self> {
        let cfg = config::load(arg(m, "config")).map_err(|e| exit::err(Code::Config, e.to_string()))?;
        let base = arg(m, "endpoint").filter(|s| !s.is_empty()).unwrap_or(&cfg.base_url);
        let url = systemone_url(base)?;
        let original = url::Url::parse(&cfg.base_url)?;
        let target = url::Url::parse(base)?;
        let foreign = original.host_str() != target.host_str() || original.port() != target.port();
        let (auth, headers) = if foreign {
            let auth = if let Some(name) = arg(m, "endpoint-key-env").filter(|s| !s.is_empty()) {
                let value = std::env::var(name).unwrap_or_default();
                if value.trim().is_empty() {
                    return Err(usage(format!("--endpoint-key-env {name} is empty or unset")));
                }
                format!("Bearer {}", value.trim())
            } else {
                String::new()
            };
            (auth, Vec::new())
        } else {
            (cfg.auth_header, cfg.headers)
        };
        let timeout = crate::api::client::parse_duration(arg(m, "timeout").unwrap_or("1m0s"))?;
        let mut b = Client::builder();
        if !timeout.is_zero() {
            b = b.timeout(timeout);
        }
        let client = b.build()?;
        Ok(Self {
            url,
            auth,
            headers,
            client,
        })
    }
    fn send(&self, body: &Value) -> anyhow::Result<Value> {
        for attempt in 0..4 {
            let mut req = self
                .client
                .post(&self.url)
                .header("Accept", "application/json")
                .header(
                    "User-Agent",
                    format!(
                        "mixlayer-cli/{} ({}; {})",
                        env!("CARGO_PKG_VERSION"),
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    ),
                )
                .json(body);
            if !self.auth.is_empty() {
                req = req.header("Authorization", &self.auth);
            }
            for (k, v) in &self.headers {
                req = req.header(k, v);
            }
            let res=req.send().map_err(|e|exit::err(Code::Api,format!("systemone request failed: {e}\nhint: check classification service health and the response contract before retrying")))?;
            let status = res.status().as_u16();
            let after = res
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let bytes = res.bytes()?;
            if bytes.len() > MAX_BODY {
                return Err(exit::err(Code::Api, "systemone response exceeds 8 MiB"));
            }
            if status < 400 {
                let v: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| exit::err(Code::Api, "invalid systemone response JSON"))?;
                return Ok(v);
            }
            let raw = String::from_utf8_lossy(&bytes);
            let masked = if self.auth.is_empty() {
                raw.into_owned()
            } else {
                let without_header = raw.replace(&self.auth, "[REDACTED]");
                if let Some((_, secret)) = self.auth.split_once(' ').filter(|(_, secret)| !secret.is_empty()) {
                    without_header.replace(secret, "[REDACTED]")
                } else {
                    without_header
                }
            };
            if (status == 429 || status == 529) && attempt < 3 {
                let delay = if after.chars().all(|c| c.is_ascii_digit()) && !after.is_empty() {
                    match after.parse::<u64>() {
                        Ok(n) if n <= 30 => Duration::from_secs(n),
                        _ => return Err(status_error(status, &after, &masked)),
                    }
                } else if let Ok(date) = chrono::DateTime::parse_from_rfc2822(&after) {
                    let n = (date.timestamp() - chrono::Utc::now().timestamp()).max(0);
                    if n > 30 {
                        return Err(status_error(status, &after, &masked));
                    }
                    Duration::from_secs(n as u64)
                } else {
                    Duration::from_millis(250 << attempt)
                };
                if delay > Duration::from_secs(30) {
                    return Err(status_error(status, &after, &masked));
                }
                std::thread::sleep(delay);
                continue;
            }
            return Err(status_error(status, &after, &masked));
        }
        unreachable!()
    }
}
fn status_error(status: u16, after: &str, body: &str) -> anyhow::Error {
    let code = match status {
        401 | 403 => Code::Auth,
        402 => Code::Billing,
        422 => Code::Usage,
        429 => Code::RateLimited,
        _ => Code::Api,
    };
    let hint = match status {
        401 => "configure MIXLAYER_API_KEY in the environment; never pass a key on argv".into(),
        403 => "check classification model and region access for your credentials".into(),
        402 => "add credits at https://console.mixlayer.com/app/billing; do not retry until resolved".into(),
        422 => "check the model, state, questions and size limits; do not retry unchanged".into(),
        429 => format!("rate limited; honor Retry-After {after:?} before retrying"),
        529 => format!("classification capacity unavailable; retry later and honor Retry-After {after:?}"),
        _ => "check classification service health and the response contract before retrying".into(),
    };
    exit::err(
        code,
        format!(
            "POST /systemone returned HTTP {status}: {}\nhint: {hint}",
            body.chars().take(4096).collect::<String>()
        ),
    )
}
fn sorted(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let ordered: BTreeMap<_, _> = map.into_iter().collect();
            Value::Object(ordered.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}
fn canonical_questions(questions: serde_json::Map<String, Value>) -> Value {
    let mut out = serde_json::Map::new();
    let ordered: BTreeMap<_, _> = questions.into_iter().collect();
    for (id, question) in ordered {
        let mut fields = serde_json::Map::new();
        for name in ["type", "instructions", "criteria"] {
            if let Some(value) = question.get(name) {
                fields.insert(name.into(), value.clone());
            }
        }
        out.insert(id, Value::Object(fields));
    }
    Value::Object(out)
}
fn render(v: &Value) -> anyhow::Result<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Object(_) | Value::Array(_) | Value::Null => Ok(serde_json::to_string(&sorted(v.clone()))?),
        _ => anyhow::bail!("description must be text, object, array or null"),
    }
}
fn check_question(text: &str) -> anyhow::Result<()> {
    if text.trim().is_empty() || text.len() > MAX_QUESTION {
        anyhow::bail!("expanded question must be nonempty and at most 16 KiB");
    }
    Ok(())
}
fn question_type(q: &Value) -> &str {
    q["type"].as_str().unwrap_or("")
}
fn validate(body: &Value) -> anyhow::Result<()> {
    let model = body["model"].as_str().unwrap_or("");
    if model.trim().is_empty() || model.len() > 256 {
        anyhow::bail!("model must be nonempty and at most 256 bytes");
    }
    let state = &body["state"];
    if !state.is_string() && !state.is_object() && !state.is_array() {
        anyhow::bail!("state must be text, object or array");
    }
    let rendered = render(state)?;
    if rendered.trim().is_empty() || rendered.len() > MAX_STATE {
        anyhow::bail!("rendered state must be nonempty and at most 4 MiB");
    }
    let questions = body["questions"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("questions must be a JSON object"))?;
    if questions.is_empty() || questions.len() > 4096 {
        anyhow::bail!("questions must contain 1–4096 entries");
    }
    let mut branches = 0;
    for (id, q) in questions {
        if id.trim().is_empty() || id.len() > 256 {
            anyhow::bail!("question ID must be nonblank and at most 256 bytes");
        }
        let mut instructions = render(
            q.get("instructions")
                .ok_or_else(|| anyhow::anyhow!("invalid JSON description"))?,
        )?;
        let extra = match question_type(q) {
            "noul" => {
                if !q["criteria"].is_null() {
                    let criteria = q["criteria"]
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("noul criteria must be an object"))?;
                    let mut rubric = serde_json::Map::new();
                    for (k, v) in criteria {
                        if k != "true" && k != "false" {
                            anyhow::bail!("noul criteria accepts only true and false");
                        }
                        render(v)?;
                        if !v.is_null() {
                            rubric.insert(k.clone(), v.clone());
                        }
                    }
                    instructions.push_str(&format!("\n\nYes/no rubric:\n{}", render(&Value::Object(rubric))?));
                }
                check_question(&instructions)?;
                1
            }
            "choice" => {
                let c = q["criteria"]
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("choice requires 1–255 described options"))?;
                if c.is_empty() || c.len() > 255 {
                    anyhow::bail!("choice requires 1–255 described options");
                }
                let mut rubric = Vec::new();
                let ordered: BTreeMap<_, _> = c.iter().collect();
                for (k, v) in ordered {
                    if k.trim().is_empty() {
                        anyhow::bail!("choice labels must be nonblank");
                    }
                    rubric.push(format!("- {k}: {}", render(v)?));
                }
                let common = format!(
                    "{instructions}\n\nAnswer rubric (choose exactly one option):\n{}",
                    rubric.join("\n")
                );
                for k in c.keys() {
                    check_question(&format!(
                        "{common}\n\nDoes the following option correctly answer the question under the full rubric?\n{k}"
                    ))?;
                }
                c.len()
            }
            "score" => {
                let c = q["criteria"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("score requires 2–10 described levels"))?;
                if c.len() < 2 || c.len() > 10 {
                    anyhow::bail!("score requires 2–10 described levels");
                }
                let rubric = c
                    .iter()
                    .enumerate()
                    .map(|(i, v)| Ok(format!("- {i}: {}", render(v)?)))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let common = format!(
                    "{instructions}\n\nAnswer rubric (choose exactly one option):\n{}",
                    rubric.join("\n")
                );
                for i in 0..c.len() {
                    check_question(&format!(
                        "{common}\n\nDoes the following option correctly answer the question under the full rubric?\n{i}"
                    ))?;
                }
                c.len()
            }
            _ => anyhow::bail!("type must be noul, choice or score"),
        };
        branches += extra;
    }
    if branches > 4096 {
        anyhow::bail!("expanded question count exceeds 4096");
    }
    if serde_json::to_vec(body)?.len() > MAX_BODY {
        anyhow::bail!("request body exceeds 8 MiB");
    }
    Ok(())
}
fn unit(v: &Value) -> bool {
    v.as_f64().is_some_and(|n| n.is_finite() && (0.0..=1.0).contains(&n))
}
fn validate_response(req: &Value, res: &Value) -> anyhow::Result<()> {
    let q = req["questions"].as_object().expect("validated");
    let a = res["answers"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("invalid systemone response envelope"))?;
    if res["model"] != req["model"]
        || a.len() != q.len()
        || res["usage"].is_null()
        || res["usage"]["output_tokens"] != 0
    {
        anyhow::bail!("invalid systemone response envelope");
    }
    for (id, question) in q {
        let answer = &a[id];
        let typ = question_type(question);
        let mut valid = answer["type"] == typ;
        if typ == "noul" {
            valid &= unit(&answer["noul"]);
        } else {
            valid &= unit(&answer["confidence"]);
            let ps = answer["probabilities"].as_object();
            valid &= ps.is_some();
            if let Some(ps) = ps {
                let sum: f64 = ps.values().filter_map(Value::as_f64).sum();
                valid &= ps.values().all(unit) && (sum - 1.0).abs() < 1e-8;
                if typ == "choice" {
                    let criteria = question["criteria"].as_object().expect("validated");
                    valid &= criteria.len() == ps.len() && criteria.keys().all(|k| ps.contains_key(k));
                    let selected = answer["choice"].as_str().unwrap_or("");
                    valid &= ps
                        .get(selected)
                        .and_then(Value::as_f64)
                        .is_some_and(|p| ps.values().all(|v| v.as_f64().unwrap_or(2.0) <= p));
                } else if typ == "score" {
                    let criteria = question["criteria"].as_array().expect("validated");
                    let legend = answer["legend"].as_object();
                    valid &= legend.is_some_and(|l| l.len() == criteria.len());
                    valid &= ps.len() == criteria.len();
                    let mut expected = 0.0;
                    for (i, v) in criteria.iter().enumerate() {
                        let key = i.to_string();
                        let p = ps.get(&key).and_then(Value::as_f64);
                        valid &= p.is_some()
                            && legend.and_then(|l| l.get(&key)).and_then(Value::as_str) == Some(render(v)?.as_str());
                        expected += i as f64 * p.unwrap_or(0.0);
                    }
                    valid &= answer["score"].as_f64().is_some_and(|s| (s - expected).abs() < 1e-8);
                }
            }
        }
        if !valid {
            anyhow::bail!("invalid systemone answer for {id:?}");
        }
    }
    Ok(())
}
fn parse_questions(data: &[u8]) -> anyhow::Result<serde_json::Map<String, Value>> {
    struct Unique;
    impl<'de> Visitor<'de> for Unique {
        type Value = serde_json::Map<String, Value>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut out = serde_json::Map::new();
            while let Some((k, v)) = map.next_entry::<String, Value>()? {
                if out.contains_key(&k) {
                    return Err(serde::de::Error::custom(format!("duplicate question ID {k:?}")));
                }
                out.insert(k, v);
            }
            Ok(out)
        }
    }
    struct Wrapper(serde_json::Map<String, Value>);
    impl<'de> Deserialize<'de> for Wrapper {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_map(Unique).map(Wrapper)
        }
    }
    let mut de = serde_json::Deserializer::from_slice(data);
    let result = Wrapper::deserialize(&mut de).map_err(usage)?;
    de.end().map_err(usage)?;
    for (id, question) in &result.0 {
        let fields = question
            .as_object()
            .ok_or_else(|| usage(format!("question {id:?} must be an object")))?;
        if let Some(key) = fields
            .keys()
            .find(|key| !["type", "instructions", "criteria"].contains(&key.as_str()))
        {
            return Err(usage(format!("question {id:?}: unknown field {key:?}")));
        }
    }
    Ok(result.0)
}
fn questions(m: &ArgMatches, inline: bool) -> anyhow::Result<serde_json::Map<String, Value>> {
    let path = arg(m, "questions").filter(|s| !s.is_empty());
    let text = if inline {
        arg(m, "questions-json").filter(|s| !s.is_empty())
    } else {
        None
    };
    if path.is_some() && text.is_some() {
        return Err(usage("use only one of --questions and --questions-json"));
    }
    let mut q = if let Some(p) = path {
        parse_questions(&read(p).map_err(usage)?).map_err(usage)?
    } else if let Some(t) = text {
        parse_questions(t.as_bytes()).map_err(usage)?
    } else {
        serde_json::Map::new()
    };
    if inline {
        for spec in values(m, "noul") {
            let (id, instructions) = spec
                .split_once('=')
                .ok_or_else(|| usage("--noul expects id=instructions"))?;
            if q.contains_key(id) {
                return Err(usage(format!("duplicate question ID {id:?}")));
            }
            q.insert(id.into(), json!({"type":"noul","instructions":instructions}));
        }
    }
    Ok(q)
}
fn state_file(data: &[u8]) -> Value {
    let trimmed = data
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if matches!(trimmed.first(), Some(b'{' | b'['))
        && let Ok(v) = serde_json::from_slice::<Value>(&trimmed)
    {
        return v;
    }
    Value::String(String::from_utf8_lossy(data).into_owned())
}
#[derive(Clone)]
struct Gate {
    id: String,
    op: String,
    choice: String,
    number: f64,
    raw: String,
}
fn gates(m: &ArgMatches, questions: &serde_json::Map<String, Value>) -> anyhow::Result<Vec<Gate>> {
    let re = Regex::new(r"^(.+?)(>=|<=|==|>|<)(.*)$")?;
    let mut out = Vec::new();
    for spec in values(m, "fail-if") {
        let c = re.captures(&spec).ok_or_else(|| {
            usage(format!(
                "invalid --fail-if {spec:?}; use id>n, id>=n, id<n, id<=n or id==choice"
            ))
        })?;
        let id = c[1].trim();
        let q = questions
            .get(id)
            .ok_or_else(|| usage(format!("--fail-if references unknown question {id:?}")))?;
        let op = c[2].to_owned();
        let choice = c[3].to_owned();
        let number = if op == "==" {
            if question_type(q) != "choice" {
                return Err(usage("== requires a choice question"));
            }
            0.0
        } else {
            if question_type(q) != "noul" && question_type(q) != "score" {
                return Err(usage("numeric comparisons require noul or score"));
            }
            let n = choice
                .trim()
                .parse::<f64>()
                .map_err(|_| usage("--fail-if requires a finite number"))?;
            if !n.is_finite() {
                return Err(usage("--fail-if requires a finite number"));
            }
            n
        };
        out.push(Gate {
            id: id.into(),
            op,
            choice,
            number,
            raw: spec,
        });
    }
    Ok(out)
}
fn matched(g: &Gate, answer: &Value) -> bool {
    if g.op == "==" {
        return answer["choice"].as_str() == Some(&g.choice);
    }
    let n = answer[if answer["type"] == "score" { "score" } else { "noul" }].as_f64();
    n.is_some_and(|n| match g.op.as_str() {
        ">" => n > g.number,
        ">=" => n >= g.number,
        "<" => n < g.number,
        "<=" => n <= g.number,
        _ => false,
    })
}
fn execute(t: &Transport, body: &Value) -> anyhow::Result<Value> {
    let res = t.send(body)?;
    validate_response(body, &res).map_err(|e| exit::err(Code::Api, e.to_string()))?;
    Ok(res)
}
pub fn request(m: &ArgMatches, body: &Value) -> anyhow::Result<Value> {
    validate(body).map_err(usage)?;
    execute(&Transport::new(m)?, body)
}
pub fn checker_request(model: &str, prompt: &str, answer: &str) -> Value {
    let questions = json!({"correct":{"type":"noul","instructions":"Does the response fully and correctly answer the request?","criteria":{"false":"The response is incorrect, incomplete, irrelevant, unsupported, or fails a requirement of the request.","true":"The response is correct, complete, relevant, and satisfies all requirements of the request."}}});
    json!({"model":model,"state":{"request":prompt,"response":answer},"questions":canonical_questions(questions.as_object().expect("questions").clone())})
}
pub fn validate_checker(model: &str, prompt: &str) -> anyhow::Result<()> {
    validate(&checker_request(model, prompt, "pending answer")).map_err(usage)
}
pub fn decide(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    if changed(m, "state") == changed(m, "state-text") {
        return Err(usage("provide exactly one of --state or --state-text"));
    }
    if arg(m, "state") == Some("-") && arg(m, "questions") == Some("-") {
        return Err(usage("state and questions cannot both read stdin"));
    }
    let questions = questions(m, true)?;
    let gates = gates(m, &questions)?;
    let state = if changed(m, "state") {
        state_file(&read(arg(m, "state").unwrap_or("")).map_err(usage)?)
    } else {
        Value::String(arg(m, "state-text").unwrap_or("").into())
    };
    let model = model(m, "model")?;
    let body = json!({"model":model,"state":state,"questions":canonical_questions(questions)});
    validate(&body).map_err(usage)?;
    let transport = Transport::new(m)?;
    if g.dry_run {
        return print(
            &json!({"dry_run":true,"method":"POST","url":transport.url,"body":body}),
            g,
        );
    }
    let response = execute(&transport, &body)?;
    if g.json || g.agent || !std::io::stdout().is_terminal() {
        print(&response, g)?;
    } else {
        println!("ID\tTYPE\tANSWER\tCONFIDENCE");
        if let Some(a) = response["answers"].as_object() {
            let sorted: BTreeMap<_, _> = a.iter().collect();
            for (id, a) in sorted {
                let value = a
                    .get("choice")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| a.get("noul").or_else(|| a.get("score")).map(Value::to_string))
                    .unwrap_or_default();
                println!(
                    "{id}\t{}\t{value}\t{}",
                    a["type"].as_str().unwrap_or(""),
                    a.get("confidence").map_or("—".into(), Value::to_string)
                );
            }
        }
    }
    if let Some(gate) = gates.iter().find(|gate| matched(gate, &response["answers"][&gate.id])) {
        return Err(exit::err(
            Code::GateMatched,
            format!(
                "decision gate matched: {}\nhint: inspect the answers before continuing",
                gate.raw
            ),
        ));
    }
    Ok(())
}
struct Row {
    number: usize,
    state: Value,
    error: Option<String>,
}
fn row_from_value(number: usize, value: Value, field: &str) -> Row {
    if !value.is_object() {
        return Row {
            number,
            state: Value::Null,
            error: Some("JSONL row must be a JSON object".into()),
        };
    }
    let state = if field.is_empty() {
        value
    } else if let Some(v) = value.get(field) {
        v.clone()
    } else {
        return Row {
            number,
            state: Value::Null,
            error: Some(format!("missing state field {field:?}")),
        };
    };
    Row {
        number,
        state,
        error: None,
    }
}
fn next_jsonl_line(reader: &mut io::BufReader<File>) -> io::Result<Option<(Vec<u8>, bool)>> {
    let mut line = Vec::new();
    let mut oversized = false;
    let mut saw_bytes = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(saw_bytes.then_some((line, oversized)));
        }
        let end = buf.iter().position(|b| *b == b'\n');
        let n = end.unwrap_or(buf.len());
        saw_bytes = true;
        if !oversized {
            if line.len() + n > MAX_BODY {
                oversized = true;
            } else {
                line.extend_from_slice(&buf[..n]);
            }
        }
        reader.consume(n + usize::from(end.is_some()));
        if end.is_some() {
            return Ok(Some((line, oversized)));
        }
    }
}
fn row_reader(input: &str, field: &str) -> anyhow::Result<Box<dyn Iterator<Item = Row> + Send>> {
    let ext = Path::new(input)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "jsonl" {
        let mut reader = io::BufReader::new(File::open(input)?);
        let field = field.to_owned();
        let mut number = 0;
        let mut done = false;
        Ok(Box::new(std::iter::from_fn(move || {
            if done {
                return None;
            }
            let line = match next_jsonl_line(&mut reader) {
                Ok(Some(line)) => line,
                Ok(None) => {
                    done = true;
                    return None;
                }
                Err(e) => {
                    done = true;
                    number += 1;
                    return Some(Row {
                        number,
                        state: Value::Null,
                        error: Some(e.to_string()),
                    });
                }
            };
            number += 1;
            if line.1 {
                return Some(Row {
                    number,
                    state: Value::Null,
                    error: Some("JSONL row exceeds 8 MiB".into()),
                });
            }
            let value = serde_json::from_slice::<Value>(&line.0).ok();
            Some(match value {
                Some(v) => row_from_value(number, v, &field),
                None => Row {
                    number,
                    state: Value::Null,
                    error: Some("JSONL row must be a JSON object".into()),
                },
            })
        })))
    } else if ext == "csv" {
        let mut reader = csv::ReaderBuilder::new().has_headers(false).from_path(input)?;
        let header = reader.records().next().ok_or_else(|| usage("read CSV header: EOF"))??;
        let header: Vec<String> = header.iter().map(str::to_owned).collect();
        if header.iter().any(|h| h.trim().is_empty())
            || header.iter().collect::<std::collections::HashSet<_>>().len() != header.len()
        {
            return Err(usage("CSV headers must be nonblank and unique"));
        }
        let field = field.to_owned();
        let mut number = 0;
        let mut records = reader.into_records();
        Ok(Box::new(std::iter::from_fn(move || {
            let record = records.next()?;
            number += 1;
            Some(match record {
                Err(e) => Row {
                    number,
                    state: Value::Null,
                    error: Some(e.to_string()),
                },
                Ok(record) => {
                    let object: serde_json::Map<String, Value> = header
                        .iter()
                        .cloned()
                        .zip(record.iter().map(|s| Value::String(s.into())))
                        .collect();
                    row_from_value(number, Value::Object(object), &field)
                }
            })
        })))
    } else {
        Err(usage("--input must be a .jsonl or .csv file"))
    }
}
#[cfg(test)]
fn rows(input: &str, field: &str, max: usize) -> anyhow::Result<Vec<Row>> {
    Ok(row_reader(input, field)?.take(max).collect())
}
fn same_file(a: &str, b: &str) -> anyhow::Result<bool> {
    let source = fs::metadata(a)?;
    let dest = match fs::metadata(b) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(source.dev() == dest.dev() && source.ino() == dest.ino())
    }
    #[cfg(not(unix))]
    {
        Ok(fs::canonicalize(a)? == fs::canonicalize(b)?)
    }
}
pub fn batch(m: &ArgMatches, g: &Globals) -> anyhow::Result<()> {
    let input = arg(m, "input")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| usage("decide batch requires --input and --questions"))?;
    let qpath = arg(m, "questions")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| usage("decide batch requires --input and --questions"))?;
    let concurrency = arg(m, "concurrency").unwrap_or("4").parse::<usize>().map_err(usage)?;
    if !(1..=64).contains(&concurrency) {
        return Err(usage("--concurrency must be between 1 and 64"));
    }
    let max = arg(m, "max-rows").unwrap_or("10000").parse::<usize>().map_err(usage)?;
    if max == 0 {
        return Err(usage("--max-rows must be positive"));
    }
    let qs = questions(m, false)?;
    let model = model(m, "model")?;
    let template = json!({"model":model,"state":"batch validation","questions":canonical_questions(qs)});
    validate(&template).map_err(usage)?;
    let transport = Transport::new(m)?;
    let mut reader = row_reader(input, arg(m, "state-field").unwrap_or("")).map_err(usage)?;
    if g.dry_run {
        let first = reader.next().ok_or_else(|| usage("batch input has no rows"))?;
        if let Some(e) = &first.error {
            return Err(usage(format!("first row: {e}")));
        }
        let count = 1 + reader.take(max - 1).count();
        let mut body = template;
        body["state"] = sorted(first.state);
        validate(&body).map_err(usage)?;
        return print(
            &json!({"dry_run":true,"method":"POST","url":transport.url,"body":body,"rows":count}),
            g,
        );
    }
    let mut writer: Box<dyn Write> = if let Some(out) = arg(m, "out").filter(|s| !s.is_empty()) {
        if same_file(input, out)? || qpath != "-" && same_file(qpath, out)? {
            return Err(usage("--out must not overwrite an input or questions file"));
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Box::new(options.open(out)?)
    } else {
        Box::new(io::stdout().lock())
    };
    let mut ok = 0;
    let mut failed = 0;
    let mut tokens = 0u64;
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut write_error = None;
    std::thread::scope(|scope| {
        let (jobs, receiver_jobs) = std::sync::mpsc::sync_channel::<Row>(concurrency);
        let receiver_jobs = std::sync::Arc::new(std::sync::Mutex::new(receiver_jobs));
        let producer_cancelled = cancelled.clone();
        scope.spawn(move || {
            for row in reader.take(max) {
                if producer_cancelled.load(std::sync::atomic::Ordering::Relaxed) || jobs.send(row).is_err() {
                    break;
                }
            }
        });
        let (sender, receiver) = std::sync::mpsc::channel();
        for _ in 0..concurrency {
            let receiver_jobs = receiver_jobs.clone();
            let cancelled = cancelled.clone();
            let sender = sender.clone();
            let transport = transport.clone();
            let template = template.clone();
            scope.spawn(move || loop {
                if cancelled.load(std::sync::atomic::Ordering::Relaxed){break;}
                let row=receiver_jobs.lock().expect("batch jobs").recv();
                let Ok(row)=row else {break};
                let mut body=template.clone();body["state"]=sorted(row.state);
                let result=if let Some(error)=row.error {Err(usage(error))}else{validate(&body).map_err(usage).and_then(|_|execute(&transport,&body))};
                let value=match result{Ok(response)=>json!({"row":row.number,"answers":response["answers"],"usage":response["usage"]}),Err(e)=>json!({"row":row.number,"error":{"message":e.to_string(),"code":exit::code_of(&e) as u8},"usage":null})};
                if sender.send(value).is_err(){break;}
            });
        }
        drop(sender);
        for result in receiver {
            if write_error.is_some() {
                continue;
            }
            let written = (|| -> anyhow::Result<()> {
                serde_json::to_writer(&mut writer, &result)?;
                writer.write_all(b"\n")?;
                Ok(())
            })();
            if let Err(e) = written {
                write_error = Some(e);
                cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                continue;
            }
            if result.get("error").is_some() {
                failed += 1;
            } else {
                ok += 1;
                tokens += result["usage"]["input_tokens"].as_u64().unwrap_or(0);
            }
        }
    });
    eprintln!("rows ok={ok} failed={failed}; total input tokens={tokens}");
    if let Some(e) = write_error {
        return Err(e);
    }
    if failed > 0 {
        return Err(exit::err(
            Code::Partial,
            format!("{failed} batch rows failed; inspect row errors"),
        ));
    }
    if ok == 0 {
        return Err(usage("batch input has no rows"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_and_questions() {
        assert_eq!(
            state_file(b"{\"large\":9007199254740993}")["large"],
            9007199254740993u64
        );
        assert_eq!(state_file(b"123"), "123");
        for scalar in [b"true".as_slice(), b"null", b"\"quoted\"", b"{bad"] {
            assert!(state_file(scalar).is_string());
        }
        assert_eq!(state_file(b"[1,\"two\"]"), json!([1, "two"]));
        assert!(parse_questions(br#"{"q":{"type":"noul"},"q":{"type":"noul"}}"#).is_err());
        assert!(parse_questions(br#"{"q":{"type":"noul","instructions":"?","unexpected":1}}"#).is_err());
    }
    #[test]
    fn systemone_url_rules() {
        assert_eq!(
            systemone_url("https://example.invalid").unwrap(),
            "https://example.invalid/v1/systemone"
        );
        assert_eq!(
            systemone_url("https://example.invalid/v1/systemone").unwrap(),
            "https://example.invalid/v1/systemone"
        );
        assert!(systemone_url("https://user:pass@example.invalid").is_err());
    }
    #[test]
    fn checker_body() {
        let v = checker_request("fixture", "ask", "answer");
        assert_eq!(v["questions"]["correct"]["type"], "noul");
        assert_eq!(v["state"]["request"], "ask");
    }
    #[test]
    fn request_body_matches_go_golden_order() {
        let mut questions = parse_questions(
            br#"{"route":{"type":"choice","instructions":"Route?","criteria":{"review":"Review","ignore":"Ignore"}}}"#,
        )
        .unwrap();
        questions.insert("dup".into(), json!({"type":"noul","instructions":"Duplicate?"}));
        let body = json!({"model":"fixture","state":"Two charges","questions":canonical_questions(questions)});
        assert_eq!(
            body.to_string(),
            r#"{"model":"fixture","state":"Two charges","questions":{"dup":{"type":"noul","instructions":"Duplicate?"},"route":{"type":"choice","instructions":"Route?","criteria":{"review":"Review","ignore":"Ignore"}}}}"#
        );
    }
    #[test]
    fn response_fails_closed() {
        let req = json!({"model":"m","state":"s","questions":{"q":{"type":"noul","instructions":"?"}}});
        assert!(validate_response(&req, &json!({})).is_err());
        assert!(validate_response(&req,&json!({"model":"m","answers":{"q":{"type":"noul","noul":1.1}},"usage":{"input_tokens":1,"output_tokens":0}})).is_err());
    }
    #[test]
    fn request_limits_match_go() {
        let base = json!({"model":"fixture","state":"facts","questions":{"q":{"type":"noul","instructions":"?"}}});
        validate(&base).unwrap();
        let mut cases = Vec::new();
        let mut v = base.clone();
        v["questions"] = json!({});
        cases.push(v);
        let mut v = base.clone();
        v["state"] = json!(42);
        cases.push(v);
        let mut v = base.clone();
        v["state"] = json!(" ");
        cases.push(v);
        let mut v = base.clone();
        v["state"] = json!("x".repeat(MAX_STATE + 1));
        cases.push(v);
        let mut v = base.clone();
        v["state"] = json!("\u{0001}".repeat(2 * 1024 * 1024));
        cases.push(v);
        let mut v = base.clone();
        v["questions"]["q"]["instructions"] = json!(true);
        cases.push(v);
        let mut v = base.clone();
        v["questions"]["q"]["instructions"] = json!("");
        cases.push(v);
        let mut v = base.clone();
        v["questions"]["q"]["criteria"] = json!({"maybe":"yes"});
        cases.push(v);
        let mut v = base.clone();
        v["questions"]["q"] = json!({"type":"score","instructions":"x".repeat(MAX_QUESTION-1),"criteria":["a","b"]});
        cases.push(v);
        let mut v = base.clone();
        v["questions"][" "] = v["questions"]["q"].clone();
        cases.push(v);
        let mut v = base.clone();
        v["questions"]["a".repeat(257)] = v["questions"]["q"].clone();
        cases.push(v);
        for (i, case) in cases.iter().enumerate() {
            assert!(validate(case).is_err(), "case {i}");
        }
        let mut inclusive = base;
        inclusive["state"] = json!("x".repeat(MAX_STATE));
        inclusive["questions"]["q"]["instructions"] = json!("x".repeat(MAX_QUESTION));
        validate(&inclusive).unwrap();
    }
    #[test]
    fn gate_comparisons_match_go() {
        let answer = json!({"type":"noul","noul":0.8});
        for (op, number, want) in [
            (">", 0.7, true),
            (">", 0.8, false),
            (">=", 0.8, true),
            ("<", 0.9, true),
            ("<", 0.8, false),
            ("<=", 0.8, true),
        ] {
            let gate = Gate {
                id: "q".into(),
                op: op.into(),
                choice: String::new(),
                number,
                raw: String::new(),
            };
            assert_eq!(matched(&gate, &answer), want);
        }
        let gate = Gate {
            id: "c".into(),
            op: "==".into(),
            choice: "review".into(),
            number: 0.0,
            raw: String::new(),
        };
        assert!(matched(&gate, &json!({"type":"choice","choice":"review"})));
    }
    #[test]
    fn batch_readers_continue_after_local_errors() {
        let dir = tempfile::tempdir().unwrap();
        let jsonl = dir.path().join("rows.jsonl");
        fs::write(&jsonl, "bad\n{\"text\":123}\n{}\n{\"text\":\"ok\"}\n").unwrap();
        let parsed = rows(jsonl.to_str().unwrap(), "text", 100).unwrap();
        assert_eq!(parsed.len(), 4);
        assert!(parsed[0].error.is_some());
        assert!(parsed[2].error.is_some());
        assert_eq!(parsed[3].state, "ok");
        assert_eq!(rows(jsonl.to_str().unwrap(), "text", 2).unwrap().len(), 2);
        fs::write(&jsonl, format!("{}\n{{\"text\":\"ok\"}}\n", "x".repeat(MAX_BODY + 1))).unwrap();
        let parsed = rows(jsonl.to_str().unwrap(), "text", 100).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].error.as_deref(), Some("JSONL row exceeds 8 MiB"));
        assert_eq!(parsed[1].state, "ok");
        let csv = dir.path().join("rows.csv");
        fs::write(&csv, "text,id\nwrong\nok,2\n").unwrap();
        let parsed = rows(csv.to_str().unwrap(), "", 100).unwrap();
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].error.is_some());
        assert_eq!(parsed[1].state["text"], "ok");
        fs::write(&csv, "text,text\nok,2\n").unwrap();
        assert!(rows(csv.to_str().unwrap(), "", 100).is_err());
    }
}
