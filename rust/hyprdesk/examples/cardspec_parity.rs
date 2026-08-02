//! Diff the rust cardspec against the python on the REAL templates dir
//! and the REAL widgets.conf — every derived value that the host consumes.
use std::process::Command;

fn main() {
    let py = Command::new("python3")
        .args(["-c", r#"
import sys, json; sys.path.insert(0,'/home/santapong/.local/lib')
from hyprdesk import cardspec, conf
out={}
tpls = cardspec.load_templates()
for name, t in sorted(tpls.items()):
    if isinstance(t, cardspec.Template):
        out[name] = dict(label=t.label, icon=t.icon, multi=t.multi, width=t.width,
            pos=t.default_pos, action=t.action, cmd=t.source_cmd,
            builtin=t.source_builtin, interval=t.interval, nrows=len(t.rows),
            params={k: (p['type'], p['default'], p['choices']) for k,p in sorted(t.params.items())},
            preview=dict(sorted(t.preview_fields.items())))
    else:
        out[name] = {'error': True}
c = conf()
out['__instances'] = cardspec.instances(c)
for iid, tn in cardspec.instances(c):
    t = tpls.get(tn)
    if isinstance(t, cardspec.Template):
        out[f'__params_{iid}'] = dict(sorted(cardspec.instance_params(c, iid, t).items()))
out['__migrate'] = cardspec.migrate_legacy_params(c, tpls)
out['__subst'] = [cardspec.subst('a {user} b {x.1.y} c {missing} d { bad', {'user':'U'}, {'x.1.y':'F'}),
                  cardspec.subst('{a}{b}', {}, None)]
out['__fields'] = dict(sorted(cardspec.parse_fields('a=1\nb.0.c = two words \nbad line\n_x=no\nq= ').items()))
print(json.dumps(out, sort_keys=True))
"#])
        .output()
        .unwrap();
    if !py.status.success() {
        eprintln!("python failed: {}", String::from_utf8_lossy(&py.stderr));
        std::process::exit(2);
    }
    let pyv: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&py.stdout).trim()).unwrap();

    // rust side, same shape
    let mut out = serde_json::Map::new();
    let tpls = hyprdesk::cardspec::load_templates();
    let mut names: Vec<_> = tpls.keys().cloned().collect();
    names.sort();
    for name in &names {
        match &tpls[name] {
            Ok(t) => {
                let params: serde_json::Map<String, serde_json::Value> = {
                    let mut ks: Vec<_> = t.params.keys().cloned().collect();
                    ks.sort();
                    ks.iter()
                        .map(|k| {
                            let p = &t.params[k];
                            (
                                k.clone(),
                                serde_json::json!([p.ptype, p.default, p.choices]),
                            )
                        })
                        .collect()
                };
                out.insert(
                    name.clone(),
                    serde_json::json!({
                        "label": t.label, "icon": t.icon, "multi": t.multi,
                        "width": t.width, "pos": t.default_pos, "action": t.action,
                        "cmd": t.source_cmd, "builtin": t.source_builtin,
                        "interval": t.interval, "nrows": t.rows.len(),
                        "params": params,
                        "preview": t.preview_fields.iter().collect::<std::collections::BTreeMap<_,_>>(),
                    }),
                );
            }
            Err(_) => {
                out.insert(name.clone(), serde_json::json!({"error": true}));
            }
        }
    }
    let c = hyprdesk::conf_all();
    let insts = hyprdesk::cardspec::instances(&c);
    out.insert(
        "__instances".into(),
        serde_json::json!(insts.iter().map(|(a, b)| vec![a, b]).collect::<Vec<_>>()),
    );
    for (iid, tn) in &insts {
        if let Some(Ok(t)) = tpls.get(tn) {
            let p = hyprdesk::cardspec::instance_params(&c, iid, t);
            out.insert(
                format!("__params_{iid}"),
                serde_json::json!(p.iter().collect::<std::collections::BTreeMap<_, _>>()),
            );
        }
    }
    out.insert(
        "__migrate".into(),
        serde_json::json!(hyprdesk::cardspec::migrate_legacy_params(&c, &tpls)),
    );
    let mut params1 = std::collections::HashMap::new();
    params1.insert("user".to_string(), "U".to_string());
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert("x.1.y".to_string(), "F".to_string());
    out.insert(
        "__subst".into(),
        serde_json::json!([
            hyprdesk::cardspec::subst("a {user} b {x.1.y} c {missing} d { bad", &params1, Some(&fields1)),
            hyprdesk::cardspec::subst("{a}{b}", &std::collections::HashMap::new(), None),
        ]),
    );
    out.insert(
        "__fields".into(),
        serde_json::json!(hyprdesk::cardspec::parse_fields("a=1\nb.0.c = two words \nbad line\n_x=no\nq= ")
            .iter()
            .collect::<std::collections::BTreeMap<_, _>>()),
    );
    let rsv = serde_json::Value::Object(out);

    if pyv == rsv {
        println!(
            "cardspec parity: {} templates + instances + subst/fields — identical",
            names.len()
        );
    } else {
        // find the first differing top-level key for a usable report
        for (k, v) in pyv.as_object().unwrap() {
            if rsv.get(k) != Some(v) {
                println!("MISMATCH at {k:?}:\n  python: {v}\n  rust:   {:?}", rsv.get(k));
            }
        }
        std::process::exit(1);
    }
}
