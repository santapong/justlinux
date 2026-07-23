"""Widget template specs — TOML files in ~/.config/hyprcard/templates/.

A template declares a card: metadata, instantiation params, ONE data
source, and a vertical stack of rows. The row-type list is FROZEN
(plus `repeat`): a widget that doesn't fit gets a new row primitive in
rows.py, never a spec-language feature — if you need an if, you need a
row type.

Dropping a .toml here self-registers the template in the picker and in
Hypr Settings, exactly like dropping a .conf did for the conky fleet.
"""
import re
import tomllib
from pathlib import Path

TEMPLATES_DIR = Path.home() / ".config/hyprcard/templates"

ROW_TYPES = {"title", "text", "keyval", "bar", "sparkline", "graph", "hr",
             "clock", "calgrid", "notesfile", "heatmap"}
NAME_RE = re.compile(r"^[A-Za-z]\w*$")     # must survive widgets.conf keys
PARAM_TYPES = {"string", "int", "choice"}
BUILTIN_SOURCES = {"clock", "stats", "netgraph", "none"}
SUBST_RE = re.compile(r"\{([\w.]+)\}")


class TemplateError(ValueError):
    """Invalid template file — shown on the picker's red tile."""


class Template:
    def __init__(self, name, data, path=None):
        t = data.get("template")
        if not isinstance(t, dict):
            raise TemplateError("missing [template] table")
        self.name = t.get("name", name)
        if not NAME_RE.match(self.name):
            raise TemplateError(
                f"template name {self.name!r} must match [A-Za-z]\\w*")
        if self.name != name:
            raise TemplateError(
                f"template name {self.name!r} != file stem {name!r}")
        self.path = path
        self.label = t.get("label", self.name.title())
        self.icon = t.get("icon", "")
        self.description = t.get("description", "")
        self.multi = bool(t.get("multi_instance", False))
        try:
            self.width = int(t.get("default_width", 300))
        except (TypeError, ValueError) as e:
            raise TemplateError(f"default_width must be an integer: {e}")
        self.default_pos = t.get("default_pos", "top_left")

        self.params = {}
        for pname, p in (data.get("params") or {}).items():
            if not NAME_RE.match(pname):
                raise TemplateError(f"param {pname!r} must match [A-Za-z]\\w*")
            ptype = p.get("type", "string")
            if ptype not in PARAM_TYPES:
                raise TemplateError(f"param {pname}: unknown type {ptype!r}")
            self.params[pname] = {
                "label": p.get("label", pname), "type": ptype,
                "default": str(p.get("default", "")),
                "choices": [str(c) for c in p.get("choices", [])],
            }

        s = data.get("source") or {}
        self.source_cmd = s.get("cmd", "")
        self.source_builtin = s.get("builtin", "")
        try:
            self.interval = max(1, int(s.get("interval", 60)))
        except (TypeError, ValueError) as e:
            raise TemplateError(f"source.interval must be an integer: {e}")
        if self.source_builtin and self.source_builtin not in BUILTIN_SOURCES:
            raise TemplateError(f"unknown builtin source {self.source_builtin!r}")
        if not self.source_cmd and not self.source_builtin:
            self.source_builtin = "none"   # static rows (clock rows self-tick)

        self.rows = data.get("rows") or []
        if not isinstance(self.rows, list) or not self.rows:
            raise TemplateError("template needs at least one [[rows]] entry")
        self._check_rows(self.rows)

        self.preview_fields = parse_fields(
            (data.get("preview") or {}).get("fields", ""))

    def _check_rows(self, rows, depth=0):
        for r in rows:
            rtype = r.get("type")
            if rtype == "repeat":
                if depth:
                    raise TemplateError("repeat cannot nest")
                if not r.get("over"):
                    raise TemplateError("repeat needs over = \"prefix\"")
                self._check_rows(r.get("rows") or [], depth + 1)
            elif rtype not in ROW_TYPES:
                raise TemplateError(f"unknown row type {rtype!r}")


FIELD_MAX = 2000       # a runaway field value must not become a render bomb


def parse_fields(text):
    """key=value lines (key may be dotted: coin.0.name) → flat dict.
    Values are capped defensively at the seam."""
    fields = {}
    for line in str(text).splitlines():
        m = re.match(r"^([\w.]+)\s*=\s*(.*)$", line.strip())
        if m:
            fields[m.group(1)] = m.group(2)[:FIELD_MAX]
    return fields


def subst(text, params, fields=None):
    """Replace {param} / {field.path} in template strings.

    Unresolved refs render as '—' (a fetch that hasn't returned yet must
    degrade gracefully, never show raw braces)."""
    def rep(m):
        key = m.group(1)
        if key in params:
            return params[key]
        if fields and key in fields:
            return fields[key]
        return "—"
    return SUBST_RE.sub(rep, str(text))


def load_template(path):
    """Template from one .toml (raises TemplateError on any problem)."""
    try:
        data = tomllib.loads(Path(path).read_text())
    except (OSError, tomllib.TOMLDecodeError) as e:
        raise TemplateError(str(e)) from e
    return Template(Path(path).stem, data, path=str(path))


def load_templates():
    """{name: Template | TemplateError} for every file in the dir —
    broken templates surface as errors, never as silent absence."""
    out = {}
    if TEMPLATES_DIR.is_dir():
        for p in sorted(TEMPLATES_DIR.glob("*.toml")):
            try:
                out[p.stem] = load_template(p)
            except TemplateError as e:
                out[p.stem] = e
            except Exception as e:      # belt-and-braces: a broken template
                out[p.stem] = TemplateError(str(e))   # must NEVER kill the host
    return out


def instances(c):
    """[(instance_id, template_name)] from widgets.conf inst_* keys."""
    out = []
    for k, v in c.items():
        if k.startswith("inst_") and NAME_RE.match(k[5:]):
            out.append((k[5:], v))
    return sorted(out)


def instance_params(c, inst_id, template):
    """Resolved params for an instance: <id>_p_<name> keys over template
    defaults. (Legacy globals github_user/trading_coins/dev_repos are
    migrated into _p_ keys at startup by the host, then never consulted —
    a live fallback here leaked old values back when the picker CLEARED a
    Configure field.)"""
    out = {}
    for pname, p in template.params.items():
        val = c.get(f"{inst_id}_p_{pname}")
        out[pname] = str(val) if val is not None else p["default"]
    return out


LEGACY_PARAM_KEYS = {"github_user": "user", "trading_coins": "coins",
                     "dev_repos": "repos"}


def migrate_legacy_params(c, templates):
    """One-shot: fold legacy global param keys into the matching
    instances' <id>_p_<name> keys — only where the instance's template
    actually declares that param and has no explicit value yet. Returns
    {changes}, {} if nothing to do."""
    changes = {}
    for inst_id, tname in instances(c):
        t = templates.get(tname)
        if not isinstance(t, Template):
            continue
        for legacy_key, pname in LEGACY_PARAM_KEYS.items():
            if (legacy_key in c and pname in t.params
                    and f"{inst_id}_p_{pname}" not in c):
                changes[f"{inst_id}_p_{pname}"] = c[legacy_key]
    return changes
