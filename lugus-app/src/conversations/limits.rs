use super::domain::invalid;
use crate::{Limits, Result};
use lugus_agent::RunLimits;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConversationLimits {
    pub message_bytes: usize,
    pub assistant_bytes: usize,
    pub context_bytes: usize,
    pub context_messages: usize,
    pub selected_refs: usize,
    pub selected_bytes: usize,
    pub activity_events: usize,
    pub activity_bytes: usize,
    pub tool_calls: usize,
    pub tool_record_bytes: usize,
    pub tool_total_bytes: usize,
    pub open_views: usize,
    pub page_items: usize,
    pub page_bytes: usize,
    pub active_runs: usize,
    pub event_capacity: usize,
    pub runtime_close_timeout_ms: u64,
}

impl Default for ConversationLimits {
    fn default() -> Self {
        Self {
            message_bytes: 16_384,
            assistant_bytes: 32_768,
            context_bytes: 262_144,
            context_messages: 64,
            selected_refs: 32,
            selected_bytes: 131_072,
            activity_events: 512,
            activity_bytes: 1_048_576,
            tool_calls: 128,
            tool_record_bytes: 1_048_576,
            tool_total_bytes: 16_777_216,
            open_views: 64,
            page_items: 100,
            page_bytes: 1_048_576,
            active_runs: 4,
            event_capacity: 256,
            runtime_close_timeout_ms: 5_000,
        }
    }
}

pub fn default_conversation_run_limits() -> RunLimits {
    RunLimits {
        timeout: Duration::from_secs(60),
        max_tool_calls: 128,
        max_tool_result_bytes: 1_048_576,
    }
}

impl ConversationLimits {
    /// No research quotas beyond representable allocation/storage sizes.
    /// Pagination, concurrency, backpressure and shutdown grace remain finite.
    pub fn unlimited_research() -> Self {
        Self {
            message_bytes: isize::MAX as usize,
            assistant_bytes: isize::MAX as usize,
            context_bytes: isize::MAX as usize,
            context_messages: isize::MAX as usize,
            selected_refs: isize::MAX as usize,
            selected_bytes: isize::MAX as usize,
            activity_events: isize::MAX as usize,
            activity_bytes: isize::MAX as usize,
            tool_calls: isize::MAX as usize,
            tool_record_bytes: isize::MAX as usize,
            tool_total_bytes: isize::MAX as usize,
            open_views: isize::MAX as usize,
            page_bytes: isize::MAX as usize,
            ..Self::default()
        }
    }

    /// Terminal metadata is charged separately from activity so an exhausted activity budget
    /// cannot prevent recording a stable error/status. Assistant output has its own limit.
    pub const TERMINAL_METADATA_BYTES: usize = 16_384;
    pub const MAX_BYTES: usize = 1_073_741_824;
    pub const MAX_ITEMS: usize = 100_000;

    pub fn validate(&self) -> Result<()> {
        let bytes = [
            self.message_bytes,
            self.assistant_bytes,
            self.context_bytes,
            self.selected_bytes,
            self.activity_bytes,
            self.tool_record_bytes,
            self.tool_total_bytes,
            self.page_bytes,
        ];
        let items = [
            self.context_messages,
            self.selected_refs,
            self.activity_events,
            self.tool_calls,
            self.open_views,
            self.page_items,
            self.event_capacity,
        ];
        if bytes
            .into_iter()
            .all(|n| (1..=isize::MAX as usize).contains(&n))
            && items
                .into_iter()
                .all(|n| (1..=isize::MAX as usize).contains(&n))
            && (1..=Self::MAX_ITEMS).contains(&self.event_capacity)
            && (1..=Self::MAX_ITEMS).contains(&self.page_items)
            && (1..=1024).contains(&self.active_runs)
            && (1..=60_000).contains(&self.runtime_close_timeout_ms)
        {
            Ok(())
        } else {
            Err(invalid("conversation limits are outside safe bounds"))
        }
    }

    pub fn effective_run_limits(&self, requested: RunLimits, app: &Limits) -> Result<RunLimits> {
        self.validate()?;
        app.validate()?;
        if requested.timeout.is_zero()
            || (requested.timeout > Duration::from_secs(86_400)
                && requested.timeout != Duration::MAX)
            || requested.max_tool_calls == 0
            || (requested.max_tool_calls > Self::MAX_ITEMS
                && requested.max_tool_calls != isize::MAX as usize)
            || requested.max_tool_result_bytes == 0
            || (requested.max_tool_result_bytes > Self::MAX_BYTES
                && requested.max_tool_result_bytes != isize::MAX as usize)
        {
            return Err(invalid("conversation run limits are outside safe bounds"));
        }
        Ok(RunLimits {
            timeout: requested.timeout,
            max_tool_calls: requested.max_tool_calls.min(self.tool_calls),
            max_tool_result_bytes: requested
                .max_tool_result_bytes
                .min(self.tool_record_bytes)
                .min(self.tool_total_bytes)
                .min(app.max_output_bytes),
        })
    }
}
