//! Read-only tools authorized against the current run's frozen selections.
use super::*;
pub(super) const NAMES: &[&str] = &[
    "lugus_read_portfolio_snapshot",
    "lugus_read_portfolio_snapshot_page",
];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    snapshot_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    snapshot_id: String,
    section: String,
    offset: usize,
    limit: usize,
}
pub(super) fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec{name:NAMES[0].into(),description:"Read the selected frozen portfolio summary. Report its accounting date, price dates, incomplete valuation and simplified opening history. This tool never refreshes or changes holdings.".into(),input_schema:json!({"type":"object","properties":{"snapshot_id":{"type":"string"}},"required":["snapshot_id"],"additionalProperties":false})},
 ToolSpec{name:NAMES[1].into(),description:"Read a page of holdings, lots, transactions or sale matches from the selected immutable portfolio snapshot. Read remaining pages before exhaustive claims.".into(),input_schema:json!({"type":"object","properties":{"snapshot_id":{"type":"string"},"section":{"type":"string","enum":["holdings","lots","transactions","matches"]},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":200}},"required":["snapshot_id","section","offset","limit"],"additionalProperties":false})}]
}
impl ResearchExecutor {
    pub(super) async fn portfolio_call(
        &self,
        scope: &Scope,
        call: &ToolCall,
    ) -> Result<(bool, Value)> {
        if call.name == NAMES[0] {
            let r: Read = serde_json::from_value(call.arguments.clone())
                .map_err(|_| invalid("invalid snapshot arguments"))?;
            let snapshot = self
                .application
                .authorized_portfolio_snapshot(scope.clone(), r.snapshot_id)
                .await?;
            return self.value(snapshot).map(|v| (true, v));
        }
        let r: Page = serde_json::from_value(call.arguments.clone())
            .map_err(|_| invalid("invalid snapshot page arguments"))?;
        let snapshot = self
            .application
            .authorized_portfolio_snapshot(scope.clone(), r.snapshot_id.clone())
            .await?;
        let page = self
            .application
            .read_portfolio_snapshot_page(
                snapshot.conversation_id,
                r.snapshot_id,
                r.section,
                PageRequest {
                    offset: r.offset,
                    limit: r.limit,
                },
            )
            .await?;
        self.value(page).map(|v| (true, v))
    }
}
