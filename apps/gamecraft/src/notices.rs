//! What the scenario tells the player: new messages and objective changes, as lines for the
//! console, and the current message for the window's title. A real heads-up display is M10.

use gamecraft_world::{Objective, ObjectiveStatus};
use serde_json::Value;
use std::collections::BTreeMap;

/// The messages showing and the objectives' statuses already told.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notices {
    messages: Vec<String>,
    objectives: BTreeMap<String, ObjectiveStatus>,
}

impl Notices {
    /// Compares the answer of `sim.state` (its `messages`) and the running world's objectives
    /// with what was told before; returns a line for each new message and each objective that
    /// appeared or changed status. Hidden objectives are not told.
    pub fn update(&mut self, state: &Value, objectives: &[Objective]) -> Vec<String> {
        let mut lines = Vec::new();
        let showing: Vec<String> = state
            .get("messages")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|m| m.get("text").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        for text in &showing {
            if !self.messages.contains(text) {
                lines.push(format!("Message: {text}"));
            }
        }
        self.messages = showing;
        for o in objectives.iter().filter(|o| !o.hidden) {
            let told = self.objectives.insert(o.id.clone(), o.status);
            if told == Some(o.status) {
                continue;
            }
            lines.push(match o.status {
                ObjectiveStatus::Open => format!("Objective: {}", o.title),
                ObjectiveStatus::Done => format!("Objective done: {}", o.title),
                ObjectiveStatus::Failed => format!("Objective failed: {}", o.title),
            });
        }
        lines
    }

    /// The newest message showing.
    pub fn message(&self) -> Option<&str> {
        self.messages.last().map(String::as_str)
    }

    /// Forgets everything told (a restart tells the objectives again).
    pub fn clear(&mut self) {
        *self = Notices::default();
    }
}
