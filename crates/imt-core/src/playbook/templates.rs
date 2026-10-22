//! Built-in playbooks for common cases. They use only the fields every log has (level, message, exception, source),
//! so they run against any data; the user copies one into their library and adapts it.

use super::Playbook;

const TEMPLATES: &str = include_str!("templates.json");

/// The built-in templates (valid by construction: a test parses and checks them all).
pub fn templates() -> Vec<Playbook> {
    serde_json::from_str::<Vec<serde_json::Value>>(TEMPLATES)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| Playbook::parse(v).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_is_valid_and_its_queries_parse() {
        let raw: Vec<serde_json::Value> = serde_json::from_str(TEMPLATES).unwrap();
        let all = templates();
        assert_eq!(raw.len(), all.len(), "a template failed validation");
        assert!(all.len() >= 4);
        let mut ids = std::collections::BTreeSet::new();
        for p in &all {
            assert!(ids.insert(p.id.clone()), "duplicate template {}", p.id);
            for s in &p.steps {
                if let crate::playbook::Action::Query { ql }
                | crate::playbook::Action::Filter { ql }
                | crate::playbook::Action::Check { ql, .. } = &s.action
                {
                    crate::ql::parse(ql).unwrap_or_else(|e| panic!("{} / {}: {}", p.id, s.id, e.message));
                }
            }
        }
    }
}
