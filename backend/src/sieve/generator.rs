use crate::db::filters::{FilterRule, FilterCondition, FilterAction};

/// Returns true if the rule can be fully represented as a Sieve script.
/// Rules with `body` conditions or `tag` actions must stay in the IDLE path.
pub fn is_sieve_capable(rule: &FilterRule) -> bool {
    let bad_condition = rule.conditions.iter().any(|c| c.field == "body");
    let bad_action = rule.actions.iter().any(|a| a.action_type == "tag");
    !bad_condition && !bad_action
}

/// Generate a Sieve script from all enabled, Sieve-capable rules.
pub fn generate_sieve_script(rules: &[FilterRule]) -> String {
    let capable: Vec<&FilterRule> = rules
        .iter()
        .filter(|r| r.enabled && is_sieve_capable(r))
        .collect();

    if capable.is_empty() {
        return "# rav-filters\n".to_string();
    }

    let needs_fileinto = capable.iter().any(|r| {
        r.actions.iter().any(|a| a.action_type == "move" || a.action_type == "delete")
    });
    let needs_imap4flags = capable.iter().any(|r| {
        r.actions.iter().any(|a| a.action_type == "mark_read" || a.action_type == "mark_starred")
    });

    let mut out = String::new();

    let mut exts: Vec<&str> = Vec::new();
    if needs_fileinto { exts.push("fileinto"); }
    if needs_imap4flags { exts.push("imap4flags"); }

    if !exts.is_empty() {
        out.push_str("require [");
        let quoted: Vec<String> = exts.iter().map(|e| format!("\"{}\"", e)).collect();
        out.push_str(&quoted.join(", "));
        out.push_str("];\n\n");
    }

    for rule in &capable {
        let conditions: Vec<String> = rule.conditions
            .iter()
            .filter_map(condition_to_sieve)
            .collect();

        if conditions.is_empty() {
            continue;
        }

        let test = if conditions.len() == 1 {
            format!("allof({})", conditions[0])
        } else if rule.match_mode == "any" {
            format!("anyof({})", conditions.join(", "))
        } else {
            format!("allof({})", conditions.join(", "))
        };

        out.push_str(&format!("if {} {{\n", test));

        for action in &rule.actions {
            if let Some(line) = action_to_sieve(action) {
                out.push_str(&format!("    {}\n", line));
            }
        }

        if rule.stop_processing {
            out.push_str("    stop;\n");
        }

        out.push_str("}\n\n");
    }

    out
}

fn field_to_header(field: &str) -> Option<&'static str> {
    match field {
        "from" => Some("From"),
        "to" => Some("To"),
        "cc" => Some("Cc"),
        "subject" => Some("Subject"),
        _ => None,
    }
}

/// Escape a value for use inside a Sieve quoted string.
fn escape_sieve(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escape a value for use in a Sieve :matches glob (additionally escapes * and ?).
fn escape_sieve_glob(s: &str) -> String {
    escape_sieve(s).replace('*', "\\*").replace('?', "\\?")
}

fn condition_to_sieve(cond: &FilterCondition) -> Option<String> {
    match cond.field.as_str() {
        "has_attachment" => {
            return Some("header :contains \"Content-Type\" \"multipart/\"".to_string());
        }
        "is_reply" => {
            return Some("exists \"In-Reply-To\"".to_string());
        }
        "size" => {
            let n: u64 = cond.value.parse().ok()?;
            return match cond.op.as_str() {
                "greater_than" => Some(format!("size :over {}", n)),
                "less_than" => Some(format!("size :under {}", n)),
                _ => None,
            };
        }
        _ => {}
    }

    let header = field_to_header(&cond.field)?;
    let v = &cond.value;

    let test = match cond.op.as_str() {
        "contains" => format!("header :contains \"{}\" \"{}\"", header, escape_sieve(v)),
        "not_contains" => format!("not header :contains \"{}\" \"{}\"", header, escape_sieve(v)),
        "equals" => format!("header :is \"{}\" \"{}\"", header, escape_sieve(v)),
        "not_equals" => format!("not header :is \"{}\" \"{}\"", header, escape_sieve(v)),
        "starts_with" => format!("header :matches \"{}\" \"{}*\"", header, escape_sieve_glob(v)),
        "ends_with" => format!("header :matches \"{}\" \"*{}\"", header, escape_sieve_glob(v)),
        _ => return None,
    };

    Some(test)
}

fn action_to_sieve(action: &FilterAction) -> Option<String> {
    match action.action_type.as_str() {
        "mark_read" => Some("addflag \"\\\\Seen\";".to_string()),
        "mark_starred" => Some("addflag \"\\\\Flagged\";".to_string()),
        "move" => {
            let folder = action.action_value.as_deref()?;
            Some(format!("fileinto \"{}\";", escape_sieve(folder)))
        }
        "delete" => Some("fileinto \"Trash\";".to_string()),
        "forward" => {
            let addr = action.action_value.as_deref()?;
            Some(format!("redirect \"{}\";", escape_sieve(addr)))
        }
        _ => None,
    }
}
