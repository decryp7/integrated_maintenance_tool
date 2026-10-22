//! An example transform plugin: a pipeline step (`{ step = "plugin", id = "redact-emails" }` in a profile) that
//! replaces e-mail addresses in messages and text fields with `<email>`, and drops records left without a message.

use anyhow::Result;
use debrief_plugin_api::prelude::*;
use debrief_plugin_api::user_regex;

/// The transform. Transforms are deterministic and keep no shared mutable state.
pub struct RedactEmails {
    email: user_regex::Regex,
}

impl RedactEmails {
    /// The transform with its pattern compiled once.
    pub fn new() -> Result<RedactEmails> {
        Ok(RedactEmails {
            email: user_regex::compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", false)?,
        })
    }
}

impl Transform for RedactEmails {
    fn id(&self) -> &str {
        "redact-emails"
    }

    fn apply(&self, record: &mut Record, _ctx: &TransformCtx) -> Verdict {
        record.message = self.email.replace_all(&record.message, "<email>").into_owned();
        for value in record.fields.values_mut() {
            if let Some(text) = value.as_str() {
                if self.email.is_match(text) {
                    *value = self.email.replace_all(text, "<email>").into_owned().into();
                }
            }
        }
        if record.message.trim().is_empty() {
            Verdict::Drop
        } else {
            Verdict::Keep
        }
    }
}

/// The plugin.
pub struct RedactPlugin;

impl Plugin for RedactPlugin {
    fn id(&self) -> &str {
        "net.decryptology.example.transform"
    }

    fn name(&self) -> &str {
        "Redact e-mail addresses (example)"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_transform(RedactEmails::new()?);
        Ok(())
    }
}
