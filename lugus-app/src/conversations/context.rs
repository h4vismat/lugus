//! Pure bounded selection: required input plus a suffix of whole prior exchanges.
use super::{
    ContextExchange, ContextSnapshot, ConversationLimits, FrozenReference, Message, MessageRole,
    RunStatus,
    domain::{invalid, resource_limit},
};
use crate::{ErrorKind, Result, agent_contract::check_serialized_size};
use serde::Serialize;

pub const CONTEXT_POLICY: &str = "conversation-context-v1";

pub fn build_context(
    new_message: &Message,
    history_newest_first: &[ContextExchange],
    references: &[FrozenReference],
    limits: &ConversationLimits,
) -> Result<ContextSnapshot> {
    build_context_with_omitted(new_message, history_newest_first, references, 0, limits)
}

/// `omitted_earlier_messages` comes from an indexed count outside the bounded history read.
/// History contains complete exchanges ordered newest first; each exchange's messages are chronological.
pub fn build_context_with_omitted(
    new_message: &Message,
    history_newest_first: &[ContextExchange],
    references: &[FrozenReference],
    omitted_earlier_messages: usize,
    limits: &ConversationLimits,
) -> Result<ContextSnapshot> {
    limits.validate()?;
    new_message.validate(limits)?;
    if new_message.role != MessageRole::User {
        return Err(invalid("a new turn requires a user message"));
    }
    if references.len() > limits.selected_refs
        || history_newest_first.len() > limits.context_messages
    {
        return Err(resource_limit());
    }
    check_serialized_size(&references, limits.selected_bytes)?;
    for reference in references {
        reference.validate(limits)?;
    }
    let mut omitted_messages = omitted_earlier_messages;
    let mut seen_ids = std::collections::BTreeSet::new();
    for exchange in history_newest_first {
        validate_exchange(exchange, new_message)?;
        for message in &exchange.messages {
            super::domain::validate_id(&message.id)?;
            if !seen_ids.insert(message.id.as_str()) {
                return Err(invalid("history contains a duplicate message identity"));
            }
        }
        omitted_messages = omitted_messages
            .checked_add(exchange.messages.len())
            .ok_or_else(resource_limit)?;
    }
    let mut envelope = Envelope {
        policy: CONTEXT_POLICY,
        exchanges: Vec::new(),
        new_message,
        references,
        omitted_messages,
    };
    let mut context_bytes = serialized_len(&envelope, limits.context_bytes)?;
    let mut message_count = 1;
    for exchange in history_newest_first {
        if exchange.messages.len() > limits.context_messages - message_count {
            break;
        }
        let mut oversized = false;
        for message in &exchange.messages {
            match message.validate(limits) {
                Ok(()) => {}
                Err(error) if error.kind == ErrorKind::ResourceLimit => {
                    oversized = true;
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        if oversized {
            break;
        }
        let exchange_bytes = match serialized_len(exchange, limits.context_bytes) {
            Ok(bytes) => bytes,
            Err(_) => break,
        };
        let next_omitted = envelope.omitted_messages - exchange.messages.len();
        // Only the exchange array and omission integer change. Account for their actual JSON
        // encodings, then recheck the complete final envelope before allocating output.
        let candidate_bytes =
            context_bytes + exchange_bytes + usize::from(!envelope.exchanges.is_empty())
                - decimal_bytes(envelope.omitted_messages)
                + decimal_bytes(next_omitted);
        if candidate_bytes > limits.context_bytes {
            break;
        }
        envelope.exchanges.push(exchange);
        envelope.omitted_messages = next_omitted;
        context_bytes = candidate_bytes;
        message_count += exchange.messages.len();
    }
    envelope.exchanges.reverse();
    let serialized = bounded_json(&envelope, limits.context_bytes)?;
    let mut message_ids = Vec::with_capacity(message_count);
    for exchange in &envelope.exchanges {
        message_ids.extend(exchange.messages.iter().map(|message| message.id.clone()));
    }
    message_ids.push(new_message.id.clone());
    // Clone selected data only after the complete serialized envelope has passed its budget.
    Ok(ContextSnapshot {
        policy: CONTEXT_POLICY.into(),
        serialized,
        message_ids,
        references: references.to_vec(),
        omitted_messages: envelope.omitted_messages,
    })
}

#[derive(Serialize)]
struct Envelope<'a> {
    policy: &'static str,
    exchanges: Vec<&'a ContextExchange>,
    new_message: &'a Message,
    references: &'a [FrozenReference],
    omitted_messages: usize,
}

fn validate_exchange(exchange: &ContextExchange, new_message: &Message) -> Result<()> {
    let expected = if exchange.status == RunStatus::Completed {
        2
    } else {
        1
    };
    if exchange.messages.len() != expected {
        return Err(invalid(
            "history must contain whole exchanges without partial assistant text",
        ));
    }
    let first = &exchange.messages[0];
    if first.role != MessageRole::User || first.run_id == new_message.run_id {
        return Err(invalid(
            "history exchange must begin with a prior user turn",
        ));
    }
    for (index, message) in exchange.messages.iter().enumerate() {
        if message.conversation_id != new_message.conversation_id
            || message.run_id != first.run_id
            || message.id == new_message.id
            || (index == 1 && (message.role != MessageRole::Assistant || message.id == first.id))
        {
            return Err(invalid(
                "history message identity or role does not match its exchange",
            ));
        }
    }
    Ok(())
}

pub(super) fn bounded_json(value: &impl Serialize, max_bytes: usize) -> Result<String> {
    // Preflight via a counting sink before serde allocates any serialized output.
    check_serialized_size(value, max_bytes)?;
    serde_json::to_string(value).map_err(|_| invalid("conversation value could not be serialized"))
}

fn decimal_bytes(value: usize) -> usize {
    if value == 0 {
        1
    } else {
        value.ilog10() as usize + 1
    }
}

fn serialized_len(value: &impl Serialize, max_bytes: usize) -> Result<usize> {
    struct Counter {
        written: usize,
        max: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.max - self.written {
                return Err(std::io::Error::other("context limit"));
            }
            self.written += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter {
        written: 0,
        max: max_bytes,
    };
    serde_json::to_writer(&mut counter, value).map_err(|_| resource_limit())?;
    Ok(counter.written)
}
