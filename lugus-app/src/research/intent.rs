//! Untrusted interpretation proposals. Only application code resolves source identity.
use chrono::{Months, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::{AppError, ErrorKind, Result};

pub const MAX_INTENT_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Workflow {
    Conversation,
    Research,
    Prices,
    Compare,
    Clarify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectMention {
    pub text: String,
    pub exchange: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchIntent {
    pub workflow: Workflow,
    pub subjects: Vec<SubjectMention>,
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    pub clarification: Option<String>,
}

fn invalid(message: &'static str) -> AppError {
    AppError::new(ErrorKind::InvalidInput, message, false)
}

fn bounded_text(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}

impl ResearchIntent {
    pub fn validate(&self, today: NaiveDate) -> Result<()> {
        let expected = match self.workflow {
            Workflow::Conversation | Workflow::Clarify => 0,
            Workflow::Research | Workflow::Prices => 1,
            Workflow::Compare => 2,
        };
        if self.subjects.len() != expected {
            return Err(invalid("workflow has an invalid subject count"));
        }
        for subject in &self.subjects {
            let text = subject.text.trim();
            // CIK is accepted only as a literal user mention; it is never source proof.
            let explicit_cik = text
                .get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("cik:"))
                && text[4..].trim().bytes().all(|byte| byte.is_ascii_digit())
                && (1..=10).contains(&text[4..].trim().len());
            if !bounded_text(&subject.text, 256)
                || text.contains(['/', '\\', '@', '?', '#'])
                || text.to_ascii_lowercase().starts_with("www.")
                || (text.contains(':') && !explicit_cik)
            {
                return Err(invalid(
                    "subject must be a bounded company name or explicit identifier mention",
                ));
            }
            if subject.exchange.as_ref().is_some_and(|exchange| {
                !bounded_text(exchange, 32)
                    || !exchange
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
            }) {
                return Err(invalid("exchange must be a bounded exchange name"));
            }
        }
        match (self.start, self.end) {
            (None, None) => {}
            (Some(start), Some(end)) if expected > 0 => {
                if start > end
                    || end > today
                    || start
                        .checked_add_months(Months::new(120))
                        .is_none_or(|limit| end > limit)
                {
                    return Err(invalid(
                        "date range must be ordered, historical, and at most ten years",
                    ));
                }
            }
            _ => {
                return Err(invalid(
                    "dates must be paired and belong to a retrieval workflow",
                ));
            }
        }
        match (&self.clarification, self.workflow) {
            (Some(text), Workflow::Clarify) if bounded_text(text, 1024) => {}
            (None, workflow) if workflow != Workflow::Clarify => {}
            _ => {
                return Err(invalid(
                    "only clarify requires bounded nonempty clarification text",
                ));
            }
        }
        Ok(())
    }
}

pub fn parse_intent(raw: &str, today: NaiveDate) -> Result<ResearchIntent> {
    if raw.len() > MAX_INTENT_BYTES {
        return Err(AppError::new(
            ErrorKind::ResourceLimit,
            "interpretation exceeds size limit",
            false,
        ));
    }
    let intent: ResearchIntent = serde_json::from_str(raw)
        .map_err(|_| invalid("interpretation must be a single strict intent JSON object"))?;
    intent.validate(today)?;
    Ok(intent)
}
