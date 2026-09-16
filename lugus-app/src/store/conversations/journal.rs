use super::*;
fn existing_tool(tx: &Connection, run: &str, call: &str, max: usize) -> Result<Option<ToolRecord>> {
    let size:Option<i64>=tx.query_row("SELECT length(CAST(payload AS BLOB)) FROM conversation_tools WHERE run_id=?1 AND call_id=?2",params![run,call],|r|r.get(0)).optional().map_err(storage)?;
    let Some(size) = size else { return Ok(None) };
    check_size(size, max)?;
    let payload: String = tx
        .query_row(
            "SELECT payload FROM conversation_tools WHERE run_id=?1 AND call_id=?2",
            params![run, call],
            |r| r.get(0),
        )
        .map_err(storage)?;
    serde_json::from_str(&payload).map(Some).map_err(storage)
}
impl SqliteApplicationStore {
    pub(super) fn activity_append(
        &mut self,
        attempt: &RunAttempt,
        kind: &str,
        data: &str,
    ) -> Result<ActivityRecord> {
        validate_id(kind)?;
        let max = record_cap(self).min(self.conversation_limits.activity_bytes);
        let repository = self.evidence.repository_identity()?;
        let now = self.clock.now();
        bound(&(kind, data), max)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        active(
            &tx,
            &self.store_key,
            attempt,
            &repository,
            self.conversation_limits
                .page_bytes
                .min(self.limits.max_output_bytes)
                .saturating_sub(128),
        )?;
        let (count,bytes):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM conversation_activity WHERE run_id=?1",[&attempt.run_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(storage)?;
        if count < 0 || count as u64 >= self.conversation_limits.activity_events as u64 {
            return Err(limit());
        }
        check_size(bytes, self.conversation_limits.activity_bytes)?;
        let event = ActivityRecord {
            run_id: attempt.run_id.clone(),
            sequence: count as u64,
            kind: kind.into(),
            data: data.into(),
            created_at: now,
        };
        let payload = json(
            &event,
            max.min(
                self.conversation_limits
                    .activity_bytes
                    .saturating_sub(bytes as usize),
            ),
        )?;
        tx.execute(
            "INSERT INTO conversation_activity VALUES(?1,?2,?3)",
            params![attempt.run_id, count, payload],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(event)
    }
    pub(super) fn tool_begin(
        &mut self,
        attempt: &RunAttempt,
        intent: &ToolIntent,
    ) -> Result<BeginTool> {
        validate_id(&intent.call_id)?;
        validate_id(&intent.name)?;
        let max = record_cap(self).min(self.conversation_limits.tool_record_bytes);
        let intent_json = json(intent, max)?;
        if intent.result_capacity < 512 || intent.result_capacity > max {
            return Err(limit());
        }
        let repository = self.evidence.repository_identity()?;
        let now = self.clock.now();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        let run = active(
            &tx,
            &self.store_key,
            attempt,
            &repository,
            self.conversation_limits
                .page_bytes
                .min(self.limits.max_output_bytes)
                .saturating_sub(128),
        )?;
        if let Some(existing) = existing_tool(&tx, &run.id, &intent.call_id, max)? {
            if existing.intent != *intent || existing.outcome.is_none() {
                return Err(conflict());
            }
            tx.commit().map_err(storage)?;
            return Ok(BeginTool::Recorded(existing));
        }
        let (count, bytes): (i64, i64) = tx
            .query_row(
                "SELECT count(*),coalesce(sum(reserved),0) FROM conversation_tools WHERE run_id=?1",
                [&run.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(storage)?;
        if count < 0 || count as u64 >= self.conversation_limits.tool_calls as u64 {
            return Err(limit());
        }
        check_size(bytes, self.conversation_limits.tool_total_bytes)?;
        let record = ToolRecord {
            run_id: run.id,
            request_id: run.request_id,
            workspace_id: run.workspace_id,
            intent: intent.clone(),
            outcome: None,
            created_at: now,
            finished_at: None,
        };
        let payload = json(&record, max)?;
        // Reserve the whole result envelope and terminal timestamp before dispatch. The capacity bounds ToolOutcome, not raw output text.
        let reserved = if self.conversation_limits.tool_total_bytes == isize::MAX as usize {
            payload.len()
        } else {
            payload
                .len()
                .checked_add(intent.result_capacity)
                .and_then(|v| v.checked_add(64))
                .ok_or_else(limit)?
        };
        if reserved > max
            || reserved
                > self
                    .conversation_limits
                    .tool_total_bytes
                    .saturating_sub(bytes as usize)
        {
            return Err(limit());
        }
        tx.execute(
            "INSERT INTO conversation_tools VALUES(?1,?2,?3,?4,?5,0)",
            params![
                record.run_id,
                intent.call_id,
                intent_json,
                payload,
                reserved as i64
            ],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(BeginTool::Dispatch(record))
    }
    pub(super) fn tool_finish(
        &mut self,
        attempt: &RunAttempt,
        call: &str,
        outcome: &ToolOutcome,
    ) -> Result<ToolRecord> {
        validate_id(call)?;
        let max = record_cap(self).min(self.conversation_limits.tool_record_bytes);
        let repository = self.evidence.repository_identity()?;
        let now = self.clock.now();
        bound(outcome, max)?;
        let outcome = match outcome {
            ToolOutcome::Failed { error } => ToolOutcome::Failed {
                error: runs::safe_failure(error),
            },
            other => other.clone(),
        };
        bound(&outcome, max)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        active(
            &tx,
            &self.store_key,
            attempt,
            &repository,
            self.conversation_limits
                .page_bytes
                .min(self.limits.max_output_bytes)
                .saturating_sub(128),
        )?;
        let mut record = existing_tool(&tx, &attempt.run_id, call, max)?
            .ok_or_else(|| error(ErrorKind::MissingData, "tool intent does not exist"))?;
        if let Some(existing) = &record.outcome {
            if *existing != outcome {
                return Err(conflict());
            }
            tx.commit().map_err(storage)?;
            return Ok(record);
        }
        bound(&outcome, record.intent.result_capacity)?;
        record.outcome = Some(outcome);
        record.finished_at = Some(now);
        let reserved: i64 = tx
            .query_row(
                "SELECT reserved FROM conversation_tools WHERE run_id=?1 AND call_id=?2",
                params![attempt.run_id, call],
                |r| r.get(0),
            )
            .map_err(storage)?;
        check_size(reserved, max)?;
        let payload = json(
            &record,
            if self.conversation_limits.tool_total_bytes == isize::MAX as usize {
                max
            } else {
                reserved as usize
            },
        )?;
        tx.execute(
            "UPDATE conversation_tools SET payload=?3,finished=1 WHERE run_id=?1 AND call_id=?2",
            params![attempt.run_id, call, payload],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(record)
    }
}
