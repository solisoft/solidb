//! Field rules: what a collection asks of a top-level field of its
//! documents (card #848).
//!
//! A JSON Schema (`storage::schema`) validates a decoded document, so a
//! collection that has one leaves the raw write path
//! (`Collection::raw_write_ready`). A rule reads one top-level field and
//! compares it with constants: required, a type, bounds, a list of values.
//! The raw path decodes that field with those the indexes read
//! (`IndexMetaSnapshot::read_fields`) and the rule is checked on it there,
//! by the function the decoded path calls on the whole document, so both
//! give the same answer and the same error.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{DbError, DbResult};

/// The most rules a collection holds.
pub const MAX_RULES: usize = 64;

/// The type a field's value must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    String,
    /// An integer or a float.
    Number,
    /// A number without a fractional part.
    Integer,
    Bool,
    Object,
    Array,
}

impl FieldType {
    fn holds(self, value: &Value) -> bool {
        match self {
            FieldType::String => value.is_string(),
            FieldType::Number => value.is_number(),
            FieldType::Integer => {
                value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|f| f.fract() == 0.0)
            }
            FieldType::Bool => value.is_boolean(),
            FieldType::Object => value.is_object(),
            FieldType::Array => value.is_array(),
        }
    }

    fn named(self) -> &'static str {
        match self {
            FieldType::String => "a string",
            FieldType::Number => "a number",
            FieldType::Integer => "an integer",
            FieldType::Bool => "a boolean",
            FieldType::Object => "an object",
            FieldType::Array => "an array",
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// What one top-level field must hold. A field that is absent or null
/// passes every rule but `required`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldRule {
    pub field: String,
    /// Present and not null.
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<FieldType>,
    /// A number, at least this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// A number, at most this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// One of these values (strings, numbers, booleans).
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub one_of: Option<Vec<Value>>,
}

/// Two scalars are the same value: `1` and `1.0` are.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

impl FieldRule {
    /// What `doc` does not respect of this rule, if anything.
    fn broken_by(&self, doc: &Value) -> Option<String> {
        let field = &self.field;
        let value = match doc.get(field) {
            None | Some(Value::Null) => {
                return self.required.then(|| format!("'{field}' is required"));
            }
            Some(value) => value,
        };
        if let Some(kind) = self.kind {
            if !kind.holds(value) {
                return Some(format!("'{field}' must be {}", kind.named()));
            }
        }
        if self.min.is_some() || self.max.is_some() {
            let Some(number) = value.as_f64() else {
                return Some(format!("'{field}' must be a number"));
            };
            if let Some(min) = self.min.filter(|min| number < *min) {
                return Some(format!("'{field}' must be at least {min}"));
            }
            if let Some(max) = self.max.filter(|max| number > *max) {
                return Some(format!("'{field}' must be at most {max}"));
            }
        }
        if let Some(allowed) = &self.one_of {
            if !allowed.iter().any(|a| same(a, value)) {
                return Some(format!("'{field}' is not one of the values allowed"));
            }
        }
        None
    }
}

/// The first rule `doc` breaks, in the order the rules were declared.
/// `doc` holds at least the fields the rules read.
pub fn broken(rules: &[FieldRule], doc: &Value) -> Option<String> {
    rules.iter().find_map(|rule| rule.broken_by(doc))
}

/// The error a write that breaks a rule is refused with: the one a JSON
/// Schema violation gives, on either write path.
pub fn refused(what: &str) -> DbError {
    DbError::InvalidDocument(format!("Schema validation failed: {what}"))
}

/// Whether `rules` can be declared.
pub fn check_definition(rules: &[FieldRule]) -> DbResult<()> {
    let bad = |what: String| Err(DbError::BadRequest(what));
    if rules.len() > MAX_RULES {
        return bad(format!("At most {MAX_RULES} field rules"));
    }
    for (at, rule) in rules.iter().enumerate() {
        let field = &rule.field;
        if field.is_empty() || field.contains('.') || field.starts_with('_') {
            return bad(format!(
                "A rule is on a top-level field of the document, not '{field}'"
            ));
        }
        if rules[..at].iter().any(|r| &r.field == field) {
            return bad(format!("Two rules on '{field}'"));
        }
        let bounded = rule.min.is_some() || rule.max.is_some();
        if !rule.required && rule.kind.is_none() && !bounded && rule.one_of.is_none() {
            return bad(format!("The rule on '{field}' asks for nothing"));
        }
        if rule.min.is_some_and(f64::is_nan) || rule.max.is_some_and(f64::is_nan) {
            return bad(format!("The bounds of '{field}' must be numbers"));
        }
        if let (Some(min), Some(max)) = (rule.min, rule.max) {
            if min > max {
                return bad(format!("'{field}': min is above max"));
            }
        }
        let numeric = matches!(
            rule.kind,
            None | Some(FieldType::Number) | Some(FieldType::Integer)
        );
        if bounded && !numeric {
            return bad(format!("'{field}': min and max are for numbers"));
        }
        if let Some(allowed) = &rule.one_of {
            if allowed.is_empty() {
                return bad(format!("'{field}': an empty list allows no value"));
            }
            if allowed
                .iter()
                .any(|v| v.is_null() || v.is_array() || v.is_object())
            {
                return bad(format!(
                    "'{field}': the values allowed are strings, numbers or booleans"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules(body: Value) -> Vec<FieldRule> {
        serde_json::from_value(body).unwrap()
    }

    #[test]
    fn a_document_is_held_to_each_rule() {
        let rules = rules(json!([
            {"field": "name", "required": true, "type": "string"},
            {"field": "price", "type": "number", "min": 0, "max": 1000},
            {"field": "qty", "type": "integer"},
            {"field": "status", "in": ["open", "paid", 3]},
            {"field": "tags", "type": "array"},
        ]));
        check_definition(&rules).unwrap();
        let ok = [
            json!({"name": "a"}),
            json!({"name": "a", "price": 0, "qty": 2.0, "status": "paid", "tags": []}),
            json!({"name": "a", "price": 999.5, "qty": null, "status": 3.0}),
        ];
        for doc in ok {
            assert_eq!(broken(&rules, &doc), None, "{doc}");
        }
        let refused = [
            (json!({}), "'name' is required"),
            (json!({"name": null}), "'name' is required"),
            (json!({"name": 7}), "'name' must be a string"),
            (
                json!({"name": "a", "price": "9"}),
                "'price' must be a number",
            ),
            (
                json!({"name": "a", "price": -1}),
                "'price' must be at least 0",
            ),
            (
                json!({"name": "a", "price": 1000.5}),
                "'price' must be at most 1000",
            ),
            (json!({"name": "a", "qty": 1.5}), "'qty' must be an integer"),
            (
                json!({"name": "a", "status": "lost"}),
                "'status' is not one of the values allowed",
            ),
            (json!({"name": "a", "tags": {}}), "'tags' must be an array"),
            // The first rule broken, in declaration order.
            (json!({"price": -1}), "'name' is required"),
        ];
        for (doc, why) in refused {
            assert_eq!(broken(&rules, &doc).as_deref(), Some(why), "{doc}");
        }
    }

    #[test]
    fn a_definition_that_says_nothing_or_too_much_is_refused() {
        for body in [
            json!([{"field": "a"}]),
            json!([{"field": "", "required": true}]),
            json!([{"field": "a.b", "required": true}]),
            json!([{"field": "_key", "required": true}]),
            json!([{"field": "a", "required": true}, {"field": "a", "type": "string"}]),
            json!([{"field": "a", "min": 2, "max": 1}]),
            json!([{"field": "a", "type": "string", "min": 1}]),
            json!([{"field": "a", "in": []}]),
            json!([{"field": "a", "in": [[1]]}]),
        ] {
            assert!(check_definition(&rules(body.clone())).is_err(), "{body}");
        }
        // An unknown word is not a rule.
        assert!(serde_json::from_value::<Vec<FieldRule>>(
            json!([{"field": "a", "requried": true}])
        )
        .is_err());
        assert!(
            serde_json::from_value::<Vec<FieldRule>>(json!([{"field": "a", "type": "text"}]))
                .is_err()
        );
    }
}
