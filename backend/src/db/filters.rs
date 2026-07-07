use regex::Regex;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single condition within a filter rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterCondition {
    /// Message field to inspect:
    /// "from" | "to" | "cc" | "subject" | "body" |
    /// "has_attachment" | "is_reply" | "size"
    pub field: String,
    /// Comparison operator:
    /// String fields: "contains" | "not_contains" | "equals" | "not_equals"
    ///   | "starts_with" | "ends_with" | "matches_regex"
    /// Boolean fields (has_attachment, is_reply): operator ignored - presence check only.
    /// Numeric field (size): "greater_than" | "less_than" (value in bytes)
    pub op: String,
    /// Value to match against. Empty for boolean fields.
    pub value: String,
}

/// A single action to take when a rule matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterAction {
    /// "move" | "tag" | "mark_read" | "mark_starred" | "delete" | "forward"
    pub action_type: String,
    /// - move: target folder name
    /// - tag: tag id
    /// - forward: destination email address
    /// - mark_read / mark_starred / delete: null
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
    /// Whether conditions are ANDed ("all") or ORed ("any").
    pub match_mode: String,
    /// Actions to execute in order when the rule matches.
    pub actions: Vec<FilterAction>,
    /// When true, no further rules are evaluated after this one matches.
    pub stop_processing: bool,
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
    #[serde(default = "default_match_mode")]
    pub match_mode: String,
    pub actions: Vec<FilterAction>,
    #[serde(default)]
    pub stop_processing: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFilterRule {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub conditions: Option<Vec<FilterCondition>>,
    pub match_mode: Option<String>,
    pub actions: Option<Vec<FilterAction>>,
    pub stop_processing: Option<bool>,
}

/// All message fields needed to evaluate filter conditions.
pub struct MessageContext<'a> {
    pub from_address: &'a str,
    pub to_addresses: &'a str,
    pub cc_addresses: &'a str,
    pub subject: &'a str,
    /// Body preview text (snippet). Full-body conditions are matched against this.
    pub body_snippet: &'a str,
    /// Message size in bytes.
    pub size: u32,
    pub has_attachments: bool,
    /// True when the message has an In-Reply-To header (is a thread reply).
    pub is_reply: bool,
}

fn default_true() -> bool {
    true
}

fn default_match_mode() -> String {
    "all".to_string()
}

const SELECT_COLS: &str =
    "id, name, enabled, priority, conditions, action_type, action_value,
     created_at, updated_at, match_mode, stop_processing, actions";

fn parse_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<FilterRule> {
    let enabled_int: i32 = row.get(2)?;
    let conditions_json: String = row.get(4)?;
    let legacy_action_type: String = row.get(5)?;
    let legacy_action_value: Option<String> = row.get(6)?;
    let match_mode: String = row.get(9)?;
    let stop_processing_int: i32 = row.get(10)?;
    let actions_json: String = row.get(11)?;

    let conditions: Vec<FilterCondition> =
        serde_json::from_str(&conditions_json).unwrap_or_default();

    // Prefer new actions column; fall back to legacy columns for pre-migration rows.
    let actions: Vec<FilterAction> = if actions_json == "[]" || actions_json.is_empty() {
        vec![FilterAction { action_type: legacy_action_type, action_value: legacy_action_value }]
    } else {
        serde_json::from_str(&actions_json).unwrap_or_default()
    };

    Ok(FilterRule {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: enabled_int != 0,
        priority: row.get(3)?,
        conditions,
        match_mode,
        actions,
        stop_processing: stop_processing_int != 0,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub fn list_filters(conn: &Connection) -> Result<Vec<FilterRule>, String> {
    let sql = format!(
        "SELECT {SELECT_COLS} FROM filter_rules ORDER BY priority ASC, created_at ASC"
    );
    let mut stmt = conn
        .prepare(&sql)
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
    validate_actions(&data.actions)?;
    let id = Uuid::new_v4().to_string();
    let conditions_json =
        serde_json::to_string(&data.conditions).map_err(|e| format!("JSON error: {e}"))?;
    let actions_json =
        serde_json::to_string(&data.actions).map_err(|e| format!("JSON error: {e}"))?;
    let (legacy_type, legacy_value) = legacy_action(&data.actions);

    conn.execute(
        "INSERT INTO filter_rules
             (id, name, enabled, priority, conditions,
              action_type, action_value,
              match_mode, stop_processing, actions)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            id,
            data.name,
            data.enabled as i32,
            data.priority,
            conditions_json,
            legacy_type,
            legacy_value,
            data.match_mode,
            data.stop_processing as i32,
            actions_json,
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
    if let Some(ref mode) = data.match_mode {
        push!("match_mode", mode.clone());
    }
    if let Some(ref actions) = data.actions {
        validate_actions(actions)?;
        let json = serde_json::to_string(actions).map_err(|e| format!("JSON error: {e}"))?;
        push!("actions", json);
        let (lt, lv) = legacy_action(actions);
        push!("action_type", lt);
        push!("action_value", lv);
    }
    if let Some(sp) = data.stop_processing {
        push!("stop_processing", sp as i32);
    }

    if sets.is_empty() {
        return get_filter(conn, id);
    }

    sets.push("updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')".to_string());
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
    let sql = format!("SELECT {SELECT_COLS} FROM filter_rules WHERE id = ?1");
    let result = conn.query_row(&sql, params![id], parse_rule);
    match result {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to get filter: {e}")),
    }
}

/// Reorder rules by updating their priority to match the given id order.
pub fn reorder_filters(conn: &Connection, ordered_ids: &[String]) -> Result<(), String> {
    for (i, id) in ordered_ids.iter().enumerate() {
        conn.execute(
            "UPDATE filter_rules SET priority = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![i as i64, id],
        )
        .map_err(|e| format!("Failed to reorder filters: {e}"))?;
    }
    Ok(())
}

/// Evaluate all enabled filter rules against a message. Returns matching rules in priority order.
pub fn matching_rules(conn: &Connection, ctx: &MessageContext<'_>) -> Result<Vec<FilterRule>, String> {
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
        let does_match = match rule.match_mode.as_str() {
            "any" => rule.conditions.iter().any(|c| eval_condition(c, ctx)),
            _ => rule.conditions.iter().all(|c| eval_condition(c, ctx)),
        };
        if does_match {
            matched.push(rule);
        }
    }

    Ok(matched)
}

fn eval_condition(cond: &FilterCondition, ctx: &MessageContext<'_>) -> bool {
    match cond.field.as_str() {
        "from" => string_matches(ctx.from_address, &cond.op, &cond.value),
        "to" => string_matches(ctx.to_addresses, &cond.op, &cond.value),
        "cc" => string_matches(ctx.cc_addresses, &cond.op, &cond.value),
        "subject" => string_matches(ctx.subject, &cond.op, &cond.value),
        "body" => string_matches(ctx.body_snippet, &cond.op, &cond.value),
        "has_attachment" => ctx.has_attachments,
        "is_reply" => ctx.is_reply,
        "size" => {
            let threshold = cond.value.parse::<u32>().unwrap_or(0);
            match cond.op.as_str() {
                "greater_than" => ctx.size > threshold,
                "less_than" => ctx.size < threshold,
                _ => false,
            }
        }
        _ => false,
    }
}

fn string_matches(haystack: &str, op: &str, needle: &str) -> bool {
    let h = haystack.to_lowercase();
    let n = needle.to_lowercase();
    match op {
        "contains" => h.contains(&n),
        "not_contains" => !h.contains(&n),
        "equals" => h == n,
        "not_equals" => h != n,
        "starts_with" => h.starts_with(&n),
        "ends_with" => h.ends_with(&n),
        "matches_regex" => Regex::new(needle).map(|re| re.is_match(haystack)).unwrap_or(false),
        _ => false,
    }
}

fn validate_actions(actions: &[FilterAction]) -> Result<(), String> {
    if actions.is_empty() {
        return Err("At least one action is required".to_string());
    }
    for action in actions {
        validate_action(action)?;
    }
    Ok(())
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
        "forward" => {
            let addr = action.action_value.as_deref().unwrap_or("");
            if addr.is_empty() || !addr.contains('@') {
                return Err("forward action requires a valid email address".to_string());
            }
        }
        "mark_read" | "mark_starred" | "delete" => {}
        other => return Err(format!("Invalid action_type: {other}")),
    }
    Ok(())
}

/// Returns (action_type, action_value) from the first action for legacy columns.
fn legacy_action(actions: &[FilterAction]) -> (String, Option<String>) {
    actions
        .first()
        .map(|a| (a.action_type.clone(), a.action_value.clone()))
        .unwrap_or_else(|| ("mark_read".to_string(), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    fn make_rule(name: &str, field: &str, op: &str, value: &str, action_type: &str, action_value: Option<&str>) -> CreateFilterRule {
        CreateFilterRule {
            name: name.to_string(),
            enabled: true,
            priority: 0,
            conditions: vec![FilterCondition {
                field: field.to_string(),
                op: op.to_string(),
                value: value.to_string(),
            }],
            match_mode: "all".to_string(),
            actions: vec![FilterAction {
                action_type: action_type.to_string(),
                action_value: action_value.map(|s| s.to_string()),
            }],
            stop_processing: false,
        }
    }

    fn ctx<'a>(from: &'a str, to: &'a str, cc: &'a str, subject: &'a str) -> MessageContext<'a> {
        MessageContext {
            from_address: from,
            to_addresses: to,
            cc_addresses: cc,
            subject,
            body_snippet: "",
            size: 1024,
            has_attachments: false,
            is_reply: false,
        }
    }

    #[test]
    fn test_create_and_list() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("R1", "from", "contains", "spam", "move", Some("Junk"))).unwrap();
        create_filter(&conn, &make_rule("R2", "subject", "starts_with", "[AD]", "mark_read", None)).unwrap();
        let rules = list_filters(&conn).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].actions[0].action_type, "move");
    }

    #[test]
    fn test_update() {
        let conn = open_test_db();
        let rule = create_filter(&conn, &make_rule("Old", "from", "equals", "x@x.com", "mark_read", None)).unwrap();
        let updated = update_filter(&conn, &rule.id, &UpdateFilterRule {
            name: Some("New".to_string()),
            enabled: None, priority: None, conditions: None, match_mode: None,
            actions: None, stop_processing: None,
        }).unwrap().unwrap();
        assert_eq!(updated.name, "New");
    }

    #[test]
    fn test_delete() {
        let conn = open_test_db();
        let rule = create_filter(&conn, &make_rule("Del", "from", "contains", "x", "mark_read", None)).unwrap();
        assert!(delete_filter(&conn, &rule.id).unwrap());
        assert!(!delete_filter(&conn, &rule.id).unwrap());
    }

    #[test]
    fn test_match_all_logic() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("Spam", "from", "contains", "spammer", "move", Some("Junk"))).unwrap();
        let m = matching_rules(&conn, &ctx("spammer@evil.com", "me@x.com", "", "Hello")).unwrap();
        assert_eq!(m.len(), 1);
        let no = matching_rules(&conn, &ctx("friend@nice.com", "me@x.com", "", "Hello")).unwrap();
        assert_eq!(no.len(), 0);
    }

    #[test]
    fn test_match_mode_any() {
        let conn = open_test_db();
        create_filter(&conn, &CreateFilterRule {
            name: "Any".to_string(),
            enabled: true,
            priority: 0,
            conditions: vec![
                FilterCondition { field: "from".to_string(), op: "contains".to_string(), value: "alpha".to_string() },
                FilterCondition { field: "from".to_string(), op: "contains".to_string(), value: "beta".to_string() },
            ],
            match_mode: "any".to_string(),
            actions: vec![FilterAction { action_type: "mark_read".to_string(), action_value: None }],
            stop_processing: false,
        }).unwrap();

        assert_eq!(matching_rules(&conn, &ctx("alpha@x.com", "", "", "")).unwrap().len(), 1);
        assert_eq!(matching_rules(&conn, &ctx("beta@x.com", "", "", "")).unwrap().len(), 1);
        assert_eq!(matching_rules(&conn, &ctx("gamma@x.com", "", "", "")).unwrap().len(), 0);
    }

    #[test]
    fn test_cc_field() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("CC", "cc", "contains", "boss", "mark_read", None)).unwrap();
        assert_eq!(matching_rules(&conn, &MessageContext { cc_addresses: "boss@corp.com", ..ctx("a@b.com", "me@x.com", "", "hi") }).unwrap().len(), 1);
        assert_eq!(matching_rules(&conn, &ctx("a@b.com", "me@x.com", "other@x.com", "hi")).unwrap().len(), 0);
    }

    #[test]
    fn test_body_snippet() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("Body", "body", "contains", "unsubscribe", "mark_read", None)).unwrap();
        let m = matching_rules(&conn, &MessageContext {
            body_snippet: "Click here to unsubscribe from this list.",
            ..ctx("a@b.com", "me@x.com", "", "Newsletter")
        }).unwrap();
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn test_size_conditions() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("Big", "size", "greater_than", "5000000", "delete", None)).unwrap();
        let big = matching_rules(&conn, &MessageContext { size: 10_000_000, ..ctx("a@b.com", "", "", "") }).unwrap();
        assert_eq!(big.len(), 1);
        let small = matching_rules(&conn, &MessageContext { size: 1000, ..ctx("a@b.com", "", "", "") }).unwrap();
        assert_eq!(small.len(), 0);
    }

    #[test]
    fn test_is_reply_condition() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("Reply", "is_reply", "equals", "", "tag", Some("tag-id"))).unwrap();
        let reply = matching_rules(&conn, &MessageContext { is_reply: true, ..ctx("a@b.com", "", "", "") }).unwrap();
        assert_eq!(reply.len(), 1);
        let new_msg = matching_rules(&conn, &ctx("a@b.com", "", "", "")).unwrap();
        assert_eq!(new_msg.len(), 0);
    }

    #[test]
    fn test_not_operators() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("NotSpam", "from", "not_contains", "spammer", "mark_read", None)).unwrap();
        assert_eq!(matching_rules(&conn, &ctx("friend@nice.com", "", "", "")).unwrap().len(), 1);
        assert_eq!(matching_rules(&conn, &ctx("spammer@evil.com", "", "", "")).unwrap().len(), 0);
    }

    #[test]
    fn test_ends_with() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("Domain", "from", "ends_with", "@corp.com", "mark_read", None)).unwrap();
        assert_eq!(matching_rules(&conn, &ctx("alice@corp.com", "", "", "")).unwrap().len(), 1);
        assert_eq!(matching_rules(&conn, &ctx("alice@other.com", "", "", "")).unwrap().len(), 0);
    }

    #[test]
    fn test_regex_operator() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("RegexFrom", "from", "matches_regex", r"^(spam|junk)@", "delete", None)).unwrap();
        assert_eq!(matching_rules(&conn, &ctx("spam@evil.com", "", "", "")).unwrap().len(), 1);
        assert_eq!(matching_rules(&conn, &ctx("junk@evil.com", "", "", "")).unwrap().len(), 1);
        assert_eq!(matching_rules(&conn, &ctx("friend@nice.com", "", "", "")).unwrap().len(), 0);
    }

    #[test]
    fn test_multiple_actions() {
        let conn = open_test_db();
        create_filter(&conn, &CreateFilterRule {
            name: "Multi".to_string(),
            enabled: true,
            priority: 0,
            conditions: vec![FilterCondition { field: "from".to_string(), op: "contains".to_string(), value: "x".to_string() }],
            match_mode: "all".to_string(),
            actions: vec![
                FilterAction { action_type: "mark_read".to_string(), action_value: None },
                FilterAction { action_type: "mark_starred".to_string(), action_value: None },
            ],
            stop_processing: false,
        }).unwrap();
        let rules = list_filters(&conn).unwrap();
        assert_eq!(rules[0].actions.len(), 2);
    }

    #[test]
    fn test_forward_validation() {
        let conn = open_test_db();
        let ok = create_filter(&conn, &make_rule("Fwd", "subject", "contains", "urgent", "forward", Some("boss@company.com")));
        assert!(ok.is_ok());
        let bad = create_filter(&conn, &make_rule("FwdBad", "subject", "contains", "x", "forward", Some("not-an-email")));
        assert!(bad.is_err());
    }

    #[test]
    fn test_reorder() {
        let conn = open_test_db();
        let r1 = create_filter(&conn, &make_rule("A", "from", "contains", "a", "mark_read", None)).unwrap();
        let r2 = create_filter(&conn, &make_rule("B", "from", "contains", "b", "mark_read", None)).unwrap();
        reorder_filters(&conn, &[r2.id.clone(), r1.id.clone()]).unwrap();
        let rules = list_filters(&conn).unwrap();
        assert_eq!(rules[0].name, "B");
        assert_eq!(rules[1].name, "A");
    }

    #[test]
    fn test_has_attachment_condition() {
        let conn = open_test_db();
        create_filter(&conn, &make_rule("Attach", "has_attachment", "contains", "", "mark_read", None)).unwrap();
        let m = matching_rules(&conn, &MessageContext { has_attachments: true, ..ctx("a@b.com", "", "", "") }).unwrap();
        assert_eq!(m.len(), 1);
        let no = matching_rules(&conn, &ctx("a@b.com", "", "", "")).unwrap();
        assert_eq!(no.len(), 0);
    }

    #[test]
    fn test_stop_processing_halts_subsequent_rules() {
        let conn = open_test_db();
        // Rule 1 (priority 0): matches "newsletter" in from, stop_processing = true
        create_filter(&conn, &CreateFilterRule {
            name: "Stopper".to_string(),
            enabled: true,
            priority: 0,
            conditions: vec![FilterCondition { field: "from".to_string(), op: "contains".to_string(), value: "newsletter".to_string() }],
            match_mode: "all".to_string(),
            actions: vec![FilterAction { action_type: "mark_read".to_string(), action_value: None }],
            stop_processing: true,
        }).unwrap();
        // Rule 2 (priority 1): also matches "newsletter" in from
        create_filter(&conn, &CreateFilterRule {
            name: "Follower".to_string(),
            enabled: true,
            priority: 1,
            conditions: vec![FilterCondition { field: "from".to_string(), op: "contains".to_string(), value: "newsletter".to_string() }],
            match_mode: "all".to_string(),
            actions: vec![FilterAction { action_type: "delete".to_string(), action_value: None }],
            stop_processing: false,
        }).unwrap();

        let matched = matching_rules(&conn, &ctx("newsletter@spam.com", "", "", "")).unwrap();
        // Both rules technically match, but the caller is responsible for stop_processing.
        // matching_rules returns all matches; execution stops in idle.rs. Verify both are returned
        // so idle.rs can apply stop_processing correctly.
        assert_eq!(matched.len(), 2);
        assert!(matched[0].stop_processing);
        assert!(!matched[1].stop_processing);
    }

    #[test]
    fn test_invalid_regex_returns_no_match() {
        let conn = open_test_db();
        // An invalid regex pattern should not crash - it gracefully returns false (no match).
        create_filter(&conn, &make_rule("BadRegex", "from", "matches_regex", "[invalid(regex", "mark_read", None)).unwrap();
        let m = matching_rules(&conn, &ctx("anything@example.com", "", "", "")).unwrap();
        assert_eq!(m.len(), 0, "invalid regex should never match");
    }

    #[test]
    fn test_invalid_action_rejected() {
        let conn = open_test_db();
        let result = create_filter(&conn, &CreateFilterRule {
            name: "Bad".to_string(),
            enabled: true,
            priority: 0,
            conditions: vec![],
            match_mode: "all".to_string(),
            actions: vec![FilterAction { action_type: "unknown".to_string(), action_value: None }],
            stop_processing: false,
        });
        assert!(result.is_err());
    }
}
