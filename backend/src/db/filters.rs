use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single condition within a filter rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterCondition {
    /// Message field to inspect: "from" | "to" | "subject" | "has_attachment"
    pub field: String,
    /// Comparison operator: "contains" | "equals" | "starts_with"
    /// Ignored when field = "has_attachment" (presence check only).
    pub op: String,
    /// Value to match against. Empty for has_attachment.
    pub value: String,
}

/// Action to take when a rule matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterAction {
    /// "move" | "tag" | "mark_read" | "delete"
    pub action_type: String,
    /// Target folder name (move) or tag id (tag). Null for mark_read / delete.
    pub action_value: Option<String>,
}

/// A complete filter rule row.
#[derive(Debug, Clone, Serialize)]
pub struct FilterRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Lower number = evaluated first.
    pub priority: i64,
    pub conditions: Vec<FilterCondition>,
    pub action: FilterAction,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateFilterRule {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i64,
    pub conditions: Vec<FilterCondition>,
    pub action: FilterAction,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFilterRule {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub conditions: Option<Vec<FilterCondition>>,
    pub action: Option<FilterAction>,
}

fn default_true() -> bool {
    true
}

fn parse_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<FilterRule> {
    let enabled_int: i32 = row.get(2)?;
    let conditions_json: String = row.get(4)?;
    let action_type: String = row.get(5)?;
    let action_value: Option<String> = row.get(6)?;

    let conditions: Vec<FilterCondition> =
        serde_json::from_str(&conditions_json).unwrap_or_default();

    Ok(FilterRule {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: enabled_int != 0,
        priority: row.get(3)?,
        conditions,
        action: FilterAction { action_type, action_value },
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub fn list_filters(conn: &Connection) -> Result<Vec<FilterRule>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, enabled, priority, conditions, action_type, action_value,
                    created_at, updated_at
             FROM filter_rules ORDER BY priority ASC, created_at ASC",
        )
        .map_err(|e| format!("Failed to prepare list_filters: {e}"))?;

    let rows = stmt
        .query_map([], parse_rule)
        .map_err(|e| format!("Failed to query filters: {e}"))?;

    let mut rules = Vec::new();
    for row in rows {
        rules.push(row.map_err(|e| format!("Failed to read filter row: {e}"))?);
    }
    Ok(rules)
}

pub fn create_filter(conn: &Connection, data: &CreateFilterRule) -> Result<FilterRule, String> {
    validate_action(&data.action)?;
    let id = Uuid::new_v4().to_string();
    let conditions_json =
        serde_json::to_string(&data.conditions).map_err(|e| format!("JSON error: {e}"))?;

    conn.execute(
        "INSERT INTO filter_rules (id, name, enabled, priority, conditions, action_type, action_value)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            data.name,
            data.enabled as i32,
            data.priority,
            conditions_json,
            data.action.action_type,
            data.action.action_value,
        ],
    )
    .map_err(|e| format!("Failed to insert filter: {e}"))?;

    get_filter(conn, &id)?.ok_or_else(|| "Failed to retrieve created filter".to_string())
}

pub fn update_filter(
    conn: &Connection,
    id: &str,
    data: &UpdateFilterRule,
) -> Result<Option<FilterRule>, String> {
    let mut sets: Vec<String> = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1usize;

    macro_rules! push {
        ($col:expr, $val:expr) => {{
            sets.push(format!("{} = ?{idx}", $col));
            values.push(Box::new($val));
            idx += 1;
        }};
    }

    if let Some(ref name) = data.name {
        push!("name", name.clone());
    }
    if let Some(enabled) = data.enabled {
        push!("enabled", enabled as i32);
    }
    if let Some(priority) = data.priority {
        push!("priority", priority);
    }
    if let Some(ref conds) = data.conditions {
        let json = serde_json::to_string(conds).map_err(|e| format!("JSON error: {e}"))?;
        push!("conditions", json);
    }
    if let Some(ref action) = data.action {
        validate_action(action)?;
        push!("action_type", action.action_type.clone());
        push!("action_value", action.action_value.clone());
    }

    if sets.is_empty() {
        return get_filter(conn, id);
    }

    sets.push("updated_at = datetime('now')".to_string());
    let sql = format!(
        "UPDATE filter_rules SET {} WHERE id = ?{idx}",
        sets.join(", ")
    );
    values.push(Box::new(id.to_string()));
    let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
    conn.execute(&sql, refs.as_slice())
        .map_err(|e| format!("Failed to update filter: {e}"))?;

    get_filter(conn, id)
}

pub fn delete_filter(conn: &Connection, id: &str) -> Result<bool, String> {
    let n = conn
        .execute("DELETE FROM filter_rules WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete filter: {e}"))?;
    Ok(n > 0)
}

pub fn get_filter(conn: &Connection, id: &str) -> Result<Option<FilterRule>, String> {
    let result = conn.query_row(
        "SELECT id, name, enabled, priority, conditions, action_type, action_value,
                created_at, updated_at
         FROM filter_rules WHERE id = ?1",
        params![id],
        parse_rule,
    );
    match result {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to get filter: {e}")),
    }
}

/// Evaluate all enabled filter rules against a message and return matching rules in priority order.
pub fn matching_rules(
    conn: &Connection,
    from_address: &str,
    to_addresses: &str,
    subject: &str,
    has_attachments: bool,
) -> Result<Vec<FilterRule>, String> {
    let rules = list_filters(conn)?;
    let mut matched = Vec::new();

    for rule in rules {
        if !rule.enabled {
            continue;
        }
        if rule.conditions.is_empty() {
            matched.push(rule);
            continue;
        }
        let all_match = rule.conditions.iter().all(|cond| {
            match cond.field.as_str() {
                "from" => string_matches(from_address, &cond.op, &cond.value),
                "to" => string_matches(to_addresses, &cond.op, &cond.value),
                "subject" => string_matches(subject, &cond.op, &cond.value),
                "has_attachment" => has_attachments,
                _ => false,
            }
        });
        if all_match {
            matched.push(rule);
        }
    }

    Ok(matched)
}

fn string_matches(haystack: &str, op: &str, needle: &str) -> bool {
    let h = haystack.to_lowercase();
    let n = needle.to_lowercase();
    match op {
        "contains" => h.contains(&n),
        "equals" => h == n,
        "starts_with" => h.starts_with(&n),
        _ => false,
    }
}

fn validate_action(action: &FilterAction) -> Result<(), String> {
    match action.action_type.as_str() {
        "move" => {
            if action.action_value.as_deref().unwrap_or("").is_empty() {
                return Err("move action requires action_value (target folder)".to_string());
            }
        }
        "tag" => {
            if action.action_value.as_deref().unwrap_or("").is_empty() {
                return Err("tag action requires action_value (tag id)".to_string());
            }
        }
        "mark_read" | "delete" => {}
        other => return Err(format!("Invalid action_type: {other}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    fn sample_create(name: &str, field: &str, op: &str, value: &str, action_type: &str, action_value: Option<&str>) -> CreateFilterRule {
        CreateFilterRule {
            name: name.to_string(),
            enabled: true,
            priority: 0,
            conditions: vec![FilterCondition {
                field: field.to_string(),
                op: op.to_string(),
                value: value.to_string(),
            }],
            action: FilterAction {
                action_type: action_type.to_string(),
                action_value: action_value.map(|s| s.to_string()),
            },
        }
    }

    #[test]
    fn test_create_and_list_filter() {
        let conn = open_test_db();
        create_filter(&conn, &sample_create("Rule 1", "from", "contains", "spam", "move", Some("Junk"))).unwrap();
        create_filter(&conn, &sample_create("Rule 2", "subject", "starts_with", "[AD]", "mark_read", None)).unwrap();

        let rules = list_filters(&conn).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "Rule 1");
        assert_eq!(rules[0].action.action_type, "move");
    }

    #[test]
    fn test_update_filter() {
        let conn = open_test_db();
        let rule = create_filter(&conn, &sample_create("Old", "from", "equals", "x@x.com", "mark_read", None)).unwrap();
        let updated = update_filter(&conn, &rule.id, &UpdateFilterRule {
            name: Some("New".to_string()),
            enabled: None,
            priority: None,
            conditions: None,
            action: None,
        }).unwrap().unwrap();
        assert_eq!(updated.name, "New");
    }

    #[test]
    fn test_delete_filter() {
        let conn = open_test_db();
        let rule = create_filter(&conn, &sample_create("Del", "from", "contains", "x", "mark_read", None)).unwrap();
        assert!(delete_filter(&conn, &rule.id).unwrap());
        assert!(!delete_filter(&conn, &rule.id).unwrap());
    }

    #[test]
    fn test_matching_rules() {
        let conn = open_test_db();
        create_filter(&conn, &sample_create("Spam mover", "from", "contains", "spammer", "move", Some("Junk"))).unwrap();
        create_filter(&conn, &sample_create("Newsletter", "subject", "starts_with", "[news]", "mark_read", None)).unwrap();

        let matched = matching_rules(&conn, "spammer@evil.com", "me@example.com", "Hello", false).unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "Spam mover");

        let matched2 = matching_rules(&conn, "sender@news.com", "me@example.com", "[news] Weekly", false).unwrap();
        assert_eq!(matched2.len(), 1);
        assert_eq!(matched2[0].name, "Newsletter");
    }

    #[test]
    fn test_invalid_action_rejected() {
        let conn = open_test_db();
        let result = create_filter(&conn, &CreateFilterRule {
            name: "Bad".to_string(),
            enabled: true,
            priority: 0,
            conditions: vec![],
            action: FilterAction { action_type: "unknown".to_string(), action_value: None },
        });
        assert!(result.is_err());
    }
}
