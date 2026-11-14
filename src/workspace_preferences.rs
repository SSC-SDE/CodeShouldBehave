//! Account-owned presentation preferences. No review or credential data is stored here.
use crate::{PrincipalId, Storage};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;
pub const REGISTRY: &str = include_str!("workspace-registry.json");
fn registry() -> Vec<Value> {
    serde_json::from_str(REGISTRY).expect("embedded registry")
}
pub fn defaults() -> Value {
    let mut modes = serde_json::Map::new();
    for mode in ["standard", "focus"] {
        let mut visibility = serde_json::Map::new();
        let mut expanded = serde_json::Map::new();
        let mut order = json!({"left":[],"main":[],"right":[]});
        for item in registry() {
            let id = item["id"].as_str().unwrap();
            visibility.insert(id.into(), item[mode].clone());
            if item["disclosure"] == true {
                expanded.insert(
                    id.into(),
                    item.get("expandedDefault")
                        .and_then(|v| v.get(mode))
                        .cloned()
                        .unwrap_or(json!(false)),
                );
            }
            order[item["region"].as_str().unwrap()]
                .as_array_mut()
                .unwrap()
                .push(json!(id));
        }
        modes.insert(mode.into(),json!({"hldAppearance":if mode=="focus"{"studio"}else{"classic"},"visibility":visibility,"expanded":expanded,"order":order,"widths":{"left":206,"right":310},"view":if mode=="focus"{"architecture"}else{"behavior"},"level":if mode=="focus"{"hld"}else{"structural"},"comparison":"combined","filters":{"changedHld":mode=="focus","changedFunctions":mode=="focus","collapseValues":mode=="focus"}}));
    }
    json!({"version":1,"theme":"graphite","defaultMode":"standard","modes":modes})
}
fn keys(value: &Value, expected: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == expected.len() && expected.iter().all(|k| o.contains_key(*k)))
}
fn migrate_mode(mode: &mut Value, focus: bool) {
    if let Some(object) = mode.as_object_mut() {
        if !object.contains_key("hldAppearance") {
            object.insert(
                "hldAppearance".into(),
                json!(if focus { "studio" } else { "classic" }),
            );
        }
    }
    if let Some(visibility) = mode.get_mut("visibility").and_then(Value::as_object_mut) {
        if !visibility.contains_key("focusBackdrop") {
            visibility.insert("focusBackdrop".into(), json!(focus));
        }
        if !visibility.contains_key("ringBackdrop") {
            visibility.insert("ringBackdrop".into(), json!(!focus));
        }
    }
    if let Some(main) = mode
        .pointer_mut("/order/main")
        .and_then(Value::as_array_mut)
    {
        if !main.iter().any(|id| id == "focusBackdrop") {
            main.push(json!("focusBackdrop"));
        }
        if !main.iter().any(|id| id == "ringBackdrop") {
            main.push(json!("ringBackdrop"));
        }
    }
}
fn migrate(doc: &mut Value) {
    if doc["version"] != 1 {
        return;
    }
    for name in ["standard", "focus"] {
        if let Some(mode) = doc.get_mut("modes").and_then(|modes| modes.get_mut(name)) {
            migrate_mode(mode, name == "focus");
        }
    }
}
pub fn validate(doc: &Value) -> Result<(), String> {
    let bad = || "Invalid workspace preferences".to_string();
    if !(keys(doc, &["version", "defaultMode", "modes"])
        || keys(doc, &["version", "theme", "defaultMode", "modes"]))
        || (doc.get("theme").is_some()
            && !matches!(
                doc["theme"].as_str(),
                Some("graphite" | "blueprint" | "terminal" | "contrast")
            ))
        || doc["version"] != 1
        || !matches!(doc["defaultMode"].as_str(), Some("standard" | "focus"))
        || !keys(&doc["modes"], &["standard", "focus"])
    {
        return Err(bad());
    }
    let rows = registry();
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    let disclosures: Vec<&str> = rows
        .iter()
        .filter(|r| r["disclosure"] == true)
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    for name in ["standard", "focus"] {
        let mut normalized = doc["modes"][name].clone();
        migrate_mode(&mut normalized, name == "focus");
        let m = &normalized;
        if !keys(
            m,
            &[
                "hldAppearance",
                "visibility",
                "expanded",
                "order",
                "widths",
                "view",
                "level",
                "comparison",
                "filters",
            ],
        ) || !matches!(m["hldAppearance"].as_str(), Some("studio" | "classic"))
            || !keys(&m["visibility"], &ids)
            || !keys(&m["expanded"], &disclosures)
            || !keys(&m["widths"], &["left", "right"])
            || !keys(&m["order"], &["left", "main", "right"])
            || !keys(
                &m["filters"],
                &["changedHld", "changedFunctions", "collapseValues"],
            )
        {
            return Err(bad());
        }
        for field in ["visibility", "expanded", "filters"] {
            if !m[field]
                .as_object()
                .unwrap()
                .values()
                .all(Value::is_boolean)
            {
                return Err(bad());
            }
        }
        for side in ["left", "right"] {
            if !m["widths"][side]
                .as_u64()
                .is_some_and(|n| (200..=420).contains(&n))
            {
                return Err(bad());
            }
        }
        if !matches!(
            m["view"].as_str(),
            Some("behavior" | "code" | "architecture")
        ) || m["visibility"][m["view"].as_str().unwrap()] != true
            || (m["view"] == "code" && m["visibility"]["source"] != true)
            || !matches!(m["level"].as_str(), Some("structural" | "hld" | "lld"))
            || m["visibility"][m["level"].as_str().unwrap()] != true
            || !matches!(m["comparison"].as_str(), Some("combined" | "compare"))
        {
            return Err(bad());
        }
        for region in ["left", "main", "right"] {
            let expected: BTreeSet<&str> = rows
                .iter()
                .filter(|r| r["region"] == region)
                .map(|r| r["id"].as_str().unwrap())
                .collect();
            let entries = m["order"][region].as_array().ok_or_else(bad)?;
            let actual: BTreeSet<&str> = entries.iter().filter_map(Value::as_str).collect();
            if actual != expected || actual.len() != entries.len() {
                return Err(bad());
            }
        }
    }
    Ok(())
}
pub fn read(storage: &Storage, principal: &PrincipalId) -> Result<Value, String> {
    let row: Option<(i64, String)> = storage
        .connection
        .query_row(
            "SELECT revision,document FROM workspace_preferences WHERE principal=?1",
            [principal.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| "Preference read unavailable")?;
    match row {
        Some((revision, document)) => {
            let mut preferences = serde_json::from_str::<Value>(&document)
                .map_err(|_| "Invalid stored preferences")?;
            migrate(&mut preferences);
            validate(&preferences)?;
            Ok(json!({"account":principal.as_str(),"revision":revision,"preferences":preferences}))
        }
        None => Ok(json!({"account":principal.as_str(),"revision":0,"preferences":defaults()})),
    }
}
pub fn write(
    storage: &mut Storage,
    principal: &PrincipalId,
    request: &Value,
) -> Result<Value, String> {
    if !keys(request, &["revision", "preferences"]) {
        return Err("Invalid workspace preferences".into());
    }
    let revision = request["revision"]
        .as_i64()
        .filter(|v| *v >= 0 && *v < i64::MAX)
        .ok_or("Invalid preference revision")?;
    let mut preferences = request["preferences"].clone();
    migrate(&mut preferences);
    validate(&preferences)?;
    let tx = storage
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| "Preference write unavailable")?;
    let current: Option<i64> = tx
        .query_row(
            "SELECT revision FROM workspace_preferences WHERE principal=?1",
            [principal.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "Preference write unavailable")?;
    if current.unwrap_or(0) != revision {
        return Err("Preference conflict".into());
    }
    tx.execute("INSERT INTO workspace_preferences(principal,revision,document) VALUES (?1,?2,?3) ON CONFLICT(principal) DO UPDATE SET revision=excluded.revision,document=excluded.document",params![principal.as_str(),revision+1,preferences.to_string()]).map_err(|_|"Preference write unavailable")?;
    tx.commit().map_err(|_| "Preference write unavailable")?;
    Ok(json!({"account":principal.as_str(),"revision":revision+1,"preferences":preferences}))
}

#[cfg(test)]
mod tests;
