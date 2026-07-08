//! Turn a community into a title / summary / keywords.
//!
//! [`keyword_summary`] is deterministic and LLM-free (used as a fallback and in
//! tests). [`llm_summary`] asks an injected [`LLMClient`] and gracefully falls
//! back to keywords on any error or parse failure.

use super::community::Community;
use crate::error::DbResult;
use crate::server::llm_client::{LLMClient, Message};
use serde_json::Value;
use std::collections::HashMap;

pub struct Summary {
    pub title: String,
    pub summary: String,
    pub keywords: Vec<String>,
}

/// Deterministic keyword summary derived from member documents' text fields.
pub fn keyword_summary(community: &Community, member_docs: &[Value]) -> Summary {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for doc in member_docs {
        collect_words(doc, &mut |w| {
            let w = w.to_lowercase();
            if w.len() >= 4 && !is_stopword(&w) {
                *freq.entry(w).or_insert(0) += 1;
            }
        });
    }
    let mut kv: Vec<(String, usize)> = freq.into_iter().collect();
    kv.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let keywords: Vec<String> = kv.into_iter().take(8).map(|(w, _)| w).collect();

    let title = community
        .top_nodes
        .first()
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| format!("Community {}", community.id));
    let summary = if keywords.is_empty() {
        format!("A community of {} connected entities.", community.size)
    } else {
        format!(
            "A community of {} connected entities. Key topics: {}.",
            community.size,
            keywords.join(", ")
        )
    };
    Summary {
        title,
        summary,
        keywords,
    }
}

/// LLM-generated summary. On network/parse failure, falls back to the keyword
/// summary (using the raw reply as the body when available).
pub async fn llm_summary(
    client: &LLMClient,
    community: &Community,
    member_docs: &[Value],
) -> DbResult<Summary> {
    let sample: Vec<String> = member_docs
        .iter()
        .take(15)
        .map(|d| serde_json::to_string(d).unwrap_or_default())
        .collect();
    let prompt = format!(
        "You are summarizing a community of {} related entities from a knowledge graph.\n\
         Sample member documents (JSON):\n{}\n\n\
         Reply with ONLY a JSON object: {{\"title\": <short title>, \"summary\": <2-3 sentences>, \"keywords\": [<up to 8 keywords>]}}.",
        community.size,
        sample.join("\n")
    );
    let messages = vec![
        Message::system(
            "You write concise, factual community summaries and reply with strict JSON only.",
        ),
        Message::user(&prompt),
    ];
    let reply = client.chat(messages).await?;

    if let Some(parsed) = extract_json_object(&reply) {
        let title = parsed
            .get("title")
            .and_then(|v| v.as_str())
            .map(String::from);
        let summary = parsed
            .get("summary")
            .and_then(|v| v.as_str())
            .map(String::from);
        let keywords = parsed.get("keywords").and_then(|v| v.as_array()).map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
        if let (Some(title), Some(summary)) = (title, summary) {
            let fallback = keyword_summary(community, member_docs);
            return Ok(Summary {
                title,
                summary,
                keywords: keywords.unwrap_or(fallback.keywords),
            });
        }
    }

    let mut fallback = keyword_summary(community, member_docs);
    if !reply.trim().is_empty() {
        fallback.summary = reply.trim().to_string();
    }
    Ok(fallback)
}

/// Extract the outermost `{ ... }` JSON object from a possibly-noisy reply
/// (tolerates code fences and surrounding prose).
fn extract_json_object(s: &str) -> Option<Value> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&s[start..=end]).ok()
}

fn collect_words(v: &Value, f: &mut impl FnMut(&str)) {
    match v {
        Value::String(s) => {
            for word in s.split(|c: char| !c.is_alphanumeric()) {
                if !word.is_empty() {
                    f(word);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_words(x, f)),
        Value::Object(o) => {
            for (k, val) in o {
                if k.starts_with('_') {
                    continue; // skip system fields (_key/_id/_rev/…)
                }
                collect_words(val, f);
            }
        }
        _ => {}
    }
}

fn is_stopword(w: &str) -> bool {
    matches!(
        w,
        "this"
            | "that"
            | "with"
            | "from"
            | "have"
            | "were"
            | "which"
            | "their"
            | "there"
            | "about"
            | "would"
            | "these"
            | "other"
            | "into"
            | "than"
            | "then"
            | "them"
            | "some"
            | "what"
            | "when"
            | "your"
            | "http"
            | "https"
            | "true"
            | "false"
            | "null"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn community() -> Community {
        Community {
            id: 0,
            members: vec!["docs/a".into(), "docs/b".into()],
            size: 2,
            internal_edges: 1.0,
            top_nodes: vec![("docs/a".into(), 3)],
        }
    }

    #[test]
    fn keyword_summary_ranks_frequent_terms_and_skips_system_fields() {
        let docs = vec![
            json!({ "_key": "a", "title": "vector database indexing", "note": "vector search" }),
            json!({ "_key": "b", "title": "vector similarity", "tags": ["database"] }),
        ];
        let s = keyword_summary(&community(), &docs);
        assert!(s.keywords.contains(&"vector".to_string()));
        assert!(s.keywords.contains(&"database".to_string()));
        assert_eq!(s.title, "docs/a");
        assert!(s.summary.contains("2 connected entities"));
    }

    #[test]
    fn extract_json_object_tolerates_fences() {
        let v = extract_json_object("```json\n{\"title\":\"x\",\"summary\":\"y\"}\n```").unwrap();
        assert_eq!(v["title"], json!("x"));
    }
}
