//! Turning a conversation into the prompt text a model family expects. The templates embedded in GGUF
//! files are Jinja programs; instead of running them we recognise the family and render it directly,
//! which is deterministic and testable. Formats were checked against the families' published templates.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptFormat {
    /// `<|im_start|>role ... <|im_end|>` (Qwen and many others). `no_think` appends an empty thinking
    /// block so models with a thinking mode answer directly.
    ChatMl {
        no_think: bool,
    },
    /// Gemma 4: `<|turn>role ... <turn|>`.
    Gemma4,
    /// Gemma 1-3: `<start_of_turn>role ... <end_of_turn>`.
    Gemma,
    Llama3,
    /// Phi-3/4: `<|user|> ... <|end|>`.
    Phi,
}

impl PromptFormat {
    /// Picks the format from the model's chat template text, falling back to the architecture name.
    pub fn detect(architecture: &str, template: Option<&str>) -> PromptFormat {
        if let Some(t) = template {
            if t.contains("<|turn>") {
                return PromptFormat::Gemma4;
            }
            if t.contains("<start_of_turn>") {
                return PromptFormat::Gemma;
            }
            if t.contains("<|im_start|>") {
                return PromptFormat::ChatMl {
                    no_think: t.contains("<think>"),
                };
            }
            if t.contains("<|start_header_id|>") {
                return PromptFormat::Llama3;
            }
            if t.contains("<|assistant|>") && t.contains("<|end|>") {
                return PromptFormat::Phi;
            }
        }
        match architecture {
            a if a.starts_with("gemma4") => PromptFormat::Gemma4,
            a if a.starts_with("gemma") => PromptFormat::Gemma,
            a if a.starts_with("llama") => PromptFormat::Llama3,
            a if a.starts_with("phi") => PromptFormat::Phi,
            _ => PromptFormat::ChatMl { no_think: false },
        }
    }

    /// `turns` are (`"user"` | `"assistant"`, text). The BOS token, where a family needs one, is added
    /// by the tokenizer, not here.
    pub fn render(self, system: &str, turns: &[(&str, String)]) -> String {
        let mut out = String::new();
        let system = system.trim();
        match self {
            PromptFormat::ChatMl { no_think } => {
                if !system.is_empty() {
                    out.push_str(&format!("<|im_start|>system\n{system}<|im_end|>\n"));
                }
                for (role, text) in turns {
                    out.push_str(&format!("<|im_start|>{role}\n{text}<|im_end|>\n"));
                }
                out.push_str("<|im_start|>assistant\n");
                if no_think {
                    out.push_str("<think>\n\n</think>\n\n");
                }
            }
            PromptFormat::Gemma4 => {
                if !system.is_empty() {
                    out.push_str(&format!("<|turn>system\n{system}<turn|>\n"));
                }
                for (role, text) in turns {
                    let role = if role == &"assistant" { "model" } else { *role };
                    out.push_str(&format!("<|turn>{role}\n{text}<turn|>\n"));
                }
                out.push_str("<|turn>model\n<|channel>thought\n<channel|>");
            }
            PromptFormat::Gemma => {
                let mut pending_system = (!system.is_empty()).then(|| format!("{system}\n\n"));
                for (role, text) in turns {
                    let role = if role == &"assistant" { "model" } else { *role };
                    let lead = if role == "user" {
                        pending_system.take().unwrap_or_default()
                    } else {
                        String::new()
                    };
                    out.push_str(&format!("<start_of_turn>{role}\n{lead}{text}<end_of_turn>\n"));
                }
                out.push_str("<start_of_turn>model\n");
            }
            PromptFormat::Llama3 => {
                if !system.is_empty() {
                    out.push_str(&format!("<|start_header_id|>system<|end_header_id|>\n\n{system}<|eot_id|>"));
                }
                for (role, text) in turns {
                    out.push_str(&format!("<|start_header_id|>{role}<|end_header_id|>\n\n{text}<|eot_id|>"));
                }
                out.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
            }
            PromptFormat::Phi => {
                if !system.is_empty() {
                    out.push_str(&format!("<|system|>\n{system}<|end|>\n"));
                }
                for (role, text) in turns {
                    out.push_str(&format!("<|{role}|>\n{text}<|end|>\n"));
                }
                out.push_str("<|assistant|>\n");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turns() -> Vec<(&'static str, String)> {
        vec![("user", "hi".into()), ("assistant", "yo".into()), ("user", "bye".into())]
    }

    #[test]
    fn detection_prefers_the_template_then_the_architecture() {
        assert_eq!(PromptFormat::detect("x", Some("{{'<|turn>' }}")), PromptFormat::Gemma4);
        assert_eq!(PromptFormat::detect("x", Some("<start_of_turn>")), PromptFormat::Gemma);
        assert_eq!(
            PromptFormat::detect("x", Some("<|im_start|> <think>")),
            PromptFormat::ChatMl { no_think: true }
        );
        assert_eq!(
            PromptFormat::detect("x", Some("<|im_start|>")),
            PromptFormat::ChatMl { no_think: false }
        );
        assert_eq!(PromptFormat::detect("x", Some("<|start_header_id|>")), PromptFormat::Llama3);
        assert_eq!(PromptFormat::detect("x", Some("<|assistant|> <|end|>")), PromptFormat::Phi);
        assert_eq!(PromptFormat::detect("gemma4", None), PromptFormat::Gemma4);
        assert_eq!(PromptFormat::detect("gemma3", Some("unknown")), PromptFormat::Gemma);
        assert_eq!(PromptFormat::detect("llama", None), PromptFormat::Llama3);
        assert_eq!(PromptFormat::detect("phi3", None), PromptFormat::Phi);
        assert_eq!(PromptFormat::detect("qwen3", None), PromptFormat::ChatMl { no_think: false });
    }

    #[test]
    fn chatml() {
        let p = PromptFormat::ChatMl { no_think: true }.render("sys", &turns());
        assert!(p.starts_with("<|im_start|>system\nsys<|im_end|>\n<|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\nyo<|im_end|>\n"));
        assert!(p.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n"));
        let q = PromptFormat::ChatMl { no_think: false }.render(" ", &turns());
        assert!(q.starts_with("<|im_start|>user") && q.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn gemma_family() {
        let p = PromptFormat::Gemma4.render("sys", &turns());
        assert!(p.starts_with("<|turn>system\nsys<turn|>\n<|turn>user\nhi<turn|>\n<|turn>model\nyo<turn|>\n"));
        assert!(p.ends_with("<|turn>model\n<|channel>thought\n<channel|>"));
        assert!(!PromptFormat::Gemma4.render("", &turns()).contains("system"));
        let g = PromptFormat::Gemma.render("sys", &turns());
        assert!(g.starts_with("<start_of_turn>user\nsys\n\nhi<end_of_turn>\n<start_of_turn>model\nyo"));
        assert!(g.ends_with("<start_of_turn>model\n"));
        assert!(!PromptFormat::Gemma.render("", &turns()).contains("\n\nhi"));
    }

    #[test]
    fn llama_and_phi() {
        let l = PromptFormat::Llama3.render("sys", &turns());
        assert!(l.starts_with("<|start_header_id|>system<|end_header_id|>\n\nsys<|eot_id|><|start_header_id|>user"));
        assert!(l.ends_with("<|start_header_id|>assistant<|end_header_id|>\n\n"));
        assert!(!PromptFormat::Llama3.render("", &turns()).contains("system"));
        let p = PromptFormat::Phi.render("sys", &turns());
        assert!(p.starts_with("<|system|>\nsys<|end|>\n<|user|>\nhi<|end|>\n<|assistant|>\nyo<|end|>\n"));
        assert!(p.ends_with("<|assistant|>\n"));
        assert!(!PromptFormat::Phi.render("", &turns()).contains("<|system|>"));
    }
}
