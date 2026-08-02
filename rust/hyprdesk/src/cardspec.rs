//! Widget template specs — TOML files in ~/.config/hyprcard/templates/,
//! ported from lib/hyprdesk/cardspec.py. The row-type list is FROZEN
//! (plus `repeat`): a widget that doesn't fit gets a new row primitive,
//! never a spec-language feature — if you need an if, you need a row type.
//!
//! `examples/cardspec_parity.rs` loads the REAL template dir and the REAL
//! conf through both implementations and diffs every derived value.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const ROW_TYPES: [&str; 11] = [
    "title", "text", "keyval", "bar", "sparkline", "graph", "hr", "clock", "calgrid",
    "notesfile", "heatmap",
];
pub const BUILTIN_SOURCES: [&str; 4] = ["clock", "stats", "netgraph", "none"];
const FIELD_MAX: usize = 2000; // a runaway field must not become a render bomb

fn name_ok(s: &str) -> bool {
    let mut ch = s.chars();
    matches!(ch.next(), Some(c) if c.is_ascii_alphabetic())
        && ch.all(|c| c.is_alphanumeric() || c == '_')
}

fn templates_dir() -> PathBuf {
    crate::home().join(".config/hyprcard/templates")
}

#[derive(Debug, Clone)]
pub struct TemplateError(pub String);

#[derive(Debug, Clone)]
pub struct Param {
    pub label: String,
    pub ptype: String,
    pub default: String,
    pub choices: Vec<String>,
}

/// One row spec, kept as raw TOML — the renderers read ad-hoc keys and a
/// typed struct per row type would freeze what the python leaves open.
pub type RowSpec = toml::Table;

#[derive(Debug, Clone)]
pub struct Template {
    pub name: String,
    pub path: Option<String>,
    pub label: String,
    pub icon: String,
    pub description: String,
    pub multi: bool,
    pub width: i32,
    pub default_pos: String,
    pub action: String,
    pub params: HashMap<String, Param>,
    pub source_cmd: String,
    pub source_builtin: String,
    pub interval: u64,
    pub rows: Vec<RowSpec>,
    pub preview_fields: HashMap<String, String>,
}

fn s(v: Option<&toml::Value>, default: &str) -> String {
    match v {
        Some(toml::Value::String(x)) => x.clone(),
        Some(other) => other.to_string(),
        None => default.to_string(),
    }
}

fn check_rows(rows: &[RowSpec], depth: usize) -> Result<(), TemplateError> {
    for r in rows {
        let rtype = r.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if rtype == "repeat" {
            if depth > 0 {
                return Err(TemplateError("repeat cannot nest".into()));
            }
            if r.get("over").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
                return Err(TemplateError("repeat needs over = \"prefix\"".into()));
            }
            let inner: Vec<RowSpec> = r
                .get("rows")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_table().cloned()).collect())
                .unwrap_or_default();
            check_rows(&inner, depth + 1)?;
        } else if !ROW_TYPES.contains(&rtype) {
            return Err(TemplateError(format!("unknown row type {rtype:?}")));
        }
    }
    Ok(())
}

impl Template {
    pub fn parse(name: &str, data: &toml::Table, path: Option<&Path>) -> Result<Template, TemplateError> {
        let t = data
            .get("template")
            .and_then(|v| v.as_table())
            .ok_or_else(|| TemplateError("missing [template] table".into()))?;
        let tname = s(t.get("name"), name);
        if !name_ok(&tname) {
            return Err(TemplateError(format!(
                "template name {tname:?} must match [A-Za-z]\\w*"
            )));
        }
        if tname != name {
            return Err(TemplateError(format!(
                "template name {tname:?} != file stem {name:?}"
            )));
        }
        let width = match t.get("default_width") {
            None => 300,
            Some(toml::Value::Integer(i)) => *i as i32,
            Some(other) => {
                return Err(TemplateError(format!(
                    "default_width must be an integer: {other}"
                )))
            }
        };

        let mut params = HashMap::new();
        if let Some(ps) = data.get("params").and_then(|v| v.as_table()) {
            for (pname, p) in ps {
                if !name_ok(pname) {
                    return Err(TemplateError(format!(
                        "param {pname:?} must match [A-Za-z]\\w*"
                    )));
                }
                let p = p.as_table().cloned().unwrap_or_default();
                let ptype = s(p.get("type"), "string");
                if !["string", "int", "choice"].contains(&ptype.as_str()) {
                    return Err(TemplateError(format!(
                        "param {pname}: unknown type {ptype:?}"
                    )));
                }
                params.insert(
                    pname.clone(),
                    Param {
                        label: s(p.get("label"), pname),
                        ptype,
                        default: s(p.get("default"), ""),
                        choices: p
                            .get("choices")
                            .and_then(|v| v.as_array())
                            .map(|a| a.iter().map(|c| s(Some(c), "")).collect())
                            .unwrap_or_default(),
                    },
                );
            }
        }

        let src = data
            .get("source")
            .and_then(|v| v.as_table())
            .cloned()
            .unwrap_or_default();
        let source_cmd = s(src.get("cmd"), "");
        let mut source_builtin = s(src.get("builtin"), "");
        let interval = match src.get("interval") {
            None => 60,
            Some(toml::Value::Integer(i)) => (*i).max(1) as u64,
            Some(other) => {
                return Err(TemplateError(format!(
                    "source.interval must be an integer: {other}"
                )))
            }
        };
        if !source_builtin.is_empty() && !BUILTIN_SOURCES.contains(&source_builtin.as_str()) {
            return Err(TemplateError(format!(
                "unknown builtin source {source_builtin:?}"
            )));
        }
        if source_cmd.is_empty() && source_builtin.is_empty() {
            source_builtin = "none".into(); // static rows (clock rows self-tick)
        }

        let rows: Vec<RowSpec> = data
            .get("rows")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_table().cloned()).collect())
            .unwrap_or_default();
        if rows.is_empty() {
            return Err(TemplateError(
                "template needs at least one [[rows]] entry".into(),
            ));
        }
        check_rows(&rows, 0)?;

        let preview_fields = parse_fields(&s(
            data.get("preview")
                .and_then(|v| v.as_table())
                .and_then(|p| p.get("fields")),
            "",
        ));

        Ok(Template {
            name: tname.clone(),
            path: path.map(|p| p.display().to_string()),
            label: s(t.get("label"), &title_case(&tname)),
            icon: s(t.get("icon"), ""),
            description: s(t.get("description"), ""),
            multi: t.get("multi_instance").and_then(|v| v.as_bool()).unwrap_or(false),
            width,
            default_pos: s(t.get("default_pos"), "top_left"),
            action: s(t.get("action"), "").trim().to_string(),
            params,
            source_cmd,
            source_builtin,
            interval,
            rows,
            preview_fields,
        })
    }
}

/// python str.title() for the label default ("netgraph" → "Netgraph").
fn title_case(sv: &str) -> String {
    let mut out = String::new();
    let mut boundary = true;
    for c in sv.chars() {
        if c.is_alphanumeric() {
            if boundary {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            boundary = false;
        } else {
            out.push(c);
            boundary = true;
        }
    }
    out
}

/// key=value lines (key may be dotted: coin.0.name) → flat map.
pub fn parse_fields(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            continue;
        }
        let val = line[eq + 1..].trim_start();
        out.insert(
            key.to_string(),
            val.chars().take(FIELD_MAX).collect::<String>(),
        );
    }
    out
}

/// Replace {param} / {field.path}; unresolved refs render as '—'.
pub fn subst(
    text: &str,
    params: &HashMap<String, String>,
    fields: Option<&HashMap<String, String>>,
) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        if let Some(close) = tail.find('}') {
            let key = &tail[..close];
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
            {
                if let Some(v) = params.get(key) {
                    out.push_str(v);
                } else if let Some(v) = fields.and_then(|f| f.get(key)) {
                    out.push_str(v);
                } else {
                    out.push('—');
                }
                rest = &tail[close + 1..];
                continue;
            }
        }
        // no closing brace / not a ref: keep the brace literally
        out.push('{');
        rest = tail;
    }
    out.push_str(rest);
    out
}

pub fn load_template(path: &Path) -> Result<Template, TemplateError> {
    let text = std::fs::read_to_string(path).map_err(|e| TemplateError(e.to_string()))?;
    let data: toml::Table = toml::from_str(&text).map_err(|e| TemplateError(e.to_string()))?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Template::parse(&stem, &data, Some(path))
}

/// {name: Ok(Template) | Err(TemplateError)} — broken templates surface as
/// errors, never as silent absence.
pub fn load_templates() -> HashMap<String, Result<Template, TemplateError>> {
    let mut out = HashMap::new();
    let Ok(dir) = std::fs::read_dir(templates_dir()) else {
        return out;
    };
    let mut paths: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort();
    for p in paths {
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        out.insert(stem, load_template(&p));
    }
    out
}

/// [(instance_id, template_name)] from widgets.conf inst_* keys, sorted.
pub fn instances(c: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = c
        .iter()
        .filter(|(k, _)| k.starts_with("inst_") && name_ok(&k[5..]))
        .map(|(k, v)| (k[5..].to_string(), v.clone()))
        .collect();
    out.sort();
    out
}

/// Resolved params for an instance: <id>_p_<name> keys over defaults.
pub fn instance_params(
    c: &HashMap<String, String>,
    inst_id: &str,
    template: &Template,
) -> HashMap<String, String> {
    template
        .params
        .iter()
        .map(|(pname, p)| {
            let v = c
                .get(&format!("{inst_id}_p_{pname}"))
                .cloned()
                .unwrap_or_else(|| p.default.clone());
            (pname.clone(), v)
        })
        .collect()
}

const LEGACY_PARAM_KEYS: [(&str, &str); 3] = [
    ("github_user", "user"),
    ("trading_coins", "coins"),
    ("dev_repos", "repos"),
];

/// One-shot fold of legacy global params into <id>_p_<name> keys.
pub fn migrate_legacy_params(
    c: &HashMap<String, String>,
    templates: &HashMap<String, Result<Template, TemplateError>>,
) -> HashMap<String, String> {
    let mut changes = HashMap::new();
    for (inst_id, tname) in instances(c) {
        let Some(Ok(t)) = templates.get(&tname) else {
            continue;
        };
        for (legacy_key, pname) in LEGACY_PARAM_KEYS {
            let target = format!("{inst_id}_p_{pname}");
            if c.contains_key(legacy_key)
                && t.params.contains_key(pname)
                && !c.contains_key(&target)
            {
                changes.insert(target, c[legacy_key].clone());
            }
        }
    }
    changes
}
