use super::*;
fn save_run(tx: &Connection, run: &RunRecord, max: usize) -> Result<()> {
    let payload = json(run, max)?;
    tx.execute(
        "UPDATE conversation_runs SET status=?2,payload=?3 WHERE id=?1",
        params![run.id, status_name(run.status), payload],
    )
    .map_err(storage)?;
    Ok(())
}
fn status_name(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Admitted => "admitted",
        RunStatus::Running => "running",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Interrupted => "interrupted",
    }
}
pub(super) fn safe_failure(e: &AppError) -> AppError {
    AppError {
        kind: e.kind,
        message: "conversation operation did not complete successfully".into(),
        retryable: e.retryable,
        retry_after_seconds: e.retry_after_seconds,
    }
}
fn duplicate(
    tx: &Connection,
    conversation: &str,
    request: &str,
    input: &str,
    max: usize,
) -> Result<Option<RunRecord>> {
    let row:Option<(bool,i64)>=tx.query_row("SELECT request_input=?3,length(CAST(id AS BLOB)) FROM conversation_runs WHERE conversation_id=?1 AND request=?2",params![conversation,request,input],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
    if let Some((same, size)) = row {
        if !same {
            return Err(conflict());
        }
        check_size(size, Scope::MAX_ID_BYTES)?;
        let id: String = tx
            .query_row(
                "SELECT id FROM conversation_runs WHERE conversation_id=?1 AND request=?2",
                params![conversation, request],
                |r| r.get(0),
            )
            .map_err(storage)?;
        return run_read(tx, conversation, &id, max).map(Some);
    }
    Ok(None)
}
fn history(
    tx: &Connection,
    id: &str,
    limits: &ConversationLimits,
) -> Result<(Vec<ContextExchange>, usize)> {
    let total: i64 = tx
        .query_row(
            "SELECT count(*) FROM conversation_messages WHERE conversation_id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    let mut stmt=tx.prepare("SELECT id,status,length(CAST(id AS BLOB)) FROM conversation_runs WHERE conversation_id=?1 ORDER BY rowid DESC LIMIT ?2").map_err(storage)?;
    let mut rows = stmt
        .query(params![id, limits.context_messages as i64])
        .map_err(storage)?;
    let mut exchanges = vec![];
    let mut loaded = 0usize;
    let mut bytes = 0usize;
    while let Some(row) = rows.next().map_err(storage)? {
        check_size(row.get(2).map_err(storage)?, Scope::MAX_ID_BYTES)?;
        let run: String = row.get(0).map_err(storage)?;
        let status: String = row.get(1).map_err(storage)?;
        let status = match status.as_str() {
            "completed" => RunStatus::Completed,
            "failed" => RunStatus::Failed,
            "interrupted" => RunStatus::Interrupted,
            _ => return Err(conflict()),
        };
        let size:i64=tx.query_row("SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM conversation_messages WHERE run_id=?1",[&run],|r|r.get(0)).map_err(storage)?;
        if size < 0
            || bytes
                .checked_add(size as usize)
                .is_none_or(|v| v > limits.context_bytes)
        {
            break;
        }
        bytes += size as usize;
        let mut messages_stmt = tx
            .prepare("SELECT payload FROM conversation_messages WHERE run_id=?1 ORDER BY rowid")
            .map_err(storage)?;
        let messages = messages_stmt
            .query_map([&run], |r| r.get::<_, String>(0))
            .map_err(storage)?
            .map(|s| serde_json::from_str(&s.map_err(storage)?).map_err(storage))
            .collect::<Result<Vec<Message>>>()?;
        loaded = loaded.checked_add(messages.len()).ok_or_else(limit)?;
        exchanges.push(ContextExchange { messages, status });
    }
    Ok((
        exchanges,
        (total as usize).checked_sub(loaded).ok_or_else(limit)?,
    ))
}
impl SqliteApplicationStore {
    pub(super) fn conversation_lookup(
        &self,
        request: &SendMessageRequest,
    ) -> Result<Option<RunRecord>> {
        request.validate(&self.conversation_limits)?;
        let input = json(request, self.conversation_limits.message_bytes)?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        authorize(
            &tx,
            &request.conversation_id,
            &self.evidence.repository_identity()?,
        )?;
        let result = duplicate(
            &tx,
            &request.conversation_id,
            &request.request_id,
            &input,
            record_cap(self),
        )?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }

    pub(super) fn conversation_activate(
        &mut self,
        lease: &LocalExecutionLease,
    ) -> Result<ExecutionEpoch> {
        if lease.store_key() != self.store_key {
            return Err(error(
                ErrorKind::ScopeMismatch,
                "execution lease belongs to another store",
            ));
        }
        lease.claim()?;
        let result = (|| {
            let max = record_cap(self);
            let now = self.clock.now();
            let tx = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(storage)?;
            let previous: i64 = tx
                .query_row(
                    "SELECT epoch FROM conversation_config WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            let generation = previous
                .checked_add(1)
                .filter(|v| *v > 0)
                .ok_or_else(limit)?;
            // Recovery reads one bounded run at a time, including repositories other than this handle's evidence adapter.
            loop {
                let id:Option<String>=tx.query_row("SELECT id FROM conversation_runs WHERE status IN ('admitted','running') LIMIT 1",[],|r|r.get(0)).optional().map_err(storage)?;
                let Some(id) = id else {
                    break;
                };
                validate_id(&id)?;
                let mut run: RunRecord = decode_one(
                    &tx,
                    "SELECT payload FROM conversation_runs WHERE id=?1",
                    &id,
                    max,
                )?;
                run.status = RunStatus::Interrupted;
                run.finished_at = Some(now);
                run.error = None;
                save_run(&tx, &run, max)?;
                tx.execute(
                    "UPDATE conversation_runs SET completion=?2 WHERE id=?1",
                    params![
                        id,
                        json(
                            &RunCompletion::Interrupted,
                            ConversationLimits::TERMINAL_METADATA_BYTES
                        )?
                    ],
                )
                .map_err(storage)?;
            }
            tx.execute(
                "UPDATE conversation_config SET epoch=?1 WHERE singleton=1",
                [generation],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(ExecutionEpoch {
                guard: lease.guard.clone(),
                generation: generation as u64,
            })
        })();
        if result.is_err() {
            lease.unclaim();
        }
        result
    }
    pub(super) fn conversation_admit(
        &mut self,
        epoch: &ExecutionEpoch,
        request: &SendMessageRequest,
    ) -> Result<RunRecord> {
        request.validate(&self.conversation_limits)?;
        let encoded = json(request, self.conversation_limits.message_bytes)?;
        let max = record_cap(self);
        let repository = self.evidence.repository_identity()?;
        let version: i64 = self
            .connection
            .query_row("PRAGMA data_version", [], |r| r.get(0))
            .map_err(storage)?;
        let (conversation, exchanges, omitted) = {
            let tx = self.connection.unchecked_transaction().map_err(storage)?;
            fence(&tx, &self.store_key, epoch)?;
            authorize(&tx, &request.conversation_id, &repository)?;
            if let Some(run) = duplicate(
                &tx,
                &request.conversation_id,
                &request.request_id,
                &encoded,
                max,
            )? {
                tx.commit().map_err(storage)?;
                return Ok(run);
            }
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM conversation_runs WHERE conversation_id=?1 AND status IN ('admitted','running'))",[&request.conversation_id],|r|r.get(0)).map_err(storage)?;
            if exists {
                return Err(conflict());
            }
            let conversation: Conversation = decode_one(
                &tx,
                "SELECT payload FROM conversations WHERE id=?1",
                &request.conversation_id,
                max,
            )?;
            let (exchanges, omitted) =
                history(&tx, &request.conversation_id, &self.conversation_limits)?;
            tx.commit().map_err(storage)?;
            (conversation, exchanges, omitted)
        };
        let scope = Scope {
            workspace_id: conversation.workspace_id.clone(),
            request_id: request.request_id.clone(),
            run_id: None,
        };
        let mut references = Vec::with_capacity(request.selected.len());
        for selected in &request.selected {
            let frozen = self.freeze_selected(&scope, selected)?;
            references.push(frozen);
            bound(&references, self.conversation_limits.selected_bytes)?;
        }
        let run_id = self.id()?;
        let now = self.clock.now();
        let message = Message {
            id: self.id()?,
            conversation_id: conversation.id.clone(),
            run_id: run_id.clone(),
            role: MessageRole::User,
            text: request.text.clone(),
            created_at: now,
        };
        message.validate(&self.conversation_limits)?;
        let message_payload = json(&message, max)?;
        let input = build_context_with_omitted(
            &message,
            &exchanges,
            &references,
            omitted,
            &self.conversation_limits,
        )?;
        let run = RunRecord {
            id: run_id,
            conversation_id: conversation.id,
            workspace_id: conversation.workspace_id,
            request_id: request.request_id.clone(),
            user_message_id: message.id.clone(),
            epoch: epoch.generation(),
            status: RunStatus::Admitted,
            input,
            created_at: now,
            finished_at: None,
            error: None,
        };
        // Reserve terminal metadata independently of activities and ensure later status reads fit the same cap.
        let payload = json(
            &run,
            max.saturating_sub(ConversationLimits::TERMINAL_METADATA_BYTES),
        )?;
        let input_json = json(&run.input, max)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        fence(&tx, &self.store_key, epoch)?;
        authorize(&tx, &run.conversation_id, &repository)?;
        if let Some(run) = duplicate(
            &tx,
            &run.conversation_id,
            &request.request_id,
            &encoded,
            max,
        )? {
            tx.commit().map_err(storage)?;
            return Ok(run);
        }
        let current: i64 = tx
            .query_row("PRAGMA data_version", [], |r| r.get(0))
            .map_err(storage)?;
        if current != version {
            return Err(conflict());
        }
        let (count,own):(i64,bool)=tx.query_row("SELECT count(*),coalesce(max(conversation_id=?1),0) FROM conversation_runs WHERE status IN ('admitted','running')",[&run.conversation_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(storage)?;
        if own {
            return Err(conflict());
        }
        if count as u64 >= self.conversation_limits.active_runs as u64 {
            return Err(limit());
        }
        tx.execute("INSERT INTO conversation_runs(id,conversation_id,request,request_input,input,epoch,status,payload) VALUES(?1,?2,?3,?4,?5,?6,'admitted',?7)",params![run.id,run.conversation_id,run.request_id,encoded,input_json,run.epoch as i64,payload]).map_err(storage)?;
        tx.execute(
            "INSERT INTO conversation_messages VALUES(?1,?2,?3,?4)",
            params![
                message.id,
                message.conversation_id,
                message.run_id,
                message_payload
            ],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(run)
    }
    pub(super) fn conversation_start(
        &mut self,
        epoch: &ExecutionEpoch,
        conversation: &str,
        id: &str,
    ) -> Result<RunAttempt> {
        let max = record_cap(self);
        let repository = self.evidence.repository_identity()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        fence(&tx, &self.store_key, epoch)?;
        authorize(&tx, conversation, &repository)?;
        let mut run = run_read(&tx, conversation, id, max)?;
        if run.epoch != epoch.generation() || run.status != RunStatus::Admitted {
            return Err(conflict());
        }
        run.status = RunStatus::Running;
        save_run(&tx, &run, max)?;
        tx.commit().map_err(storage)?;
        Ok(RunAttempt {
            epoch: epoch.clone(),
            conversation_id: conversation.into(),
            run_id: id.into(),
        })
    }
    pub(super) fn conversation_fail_admission(
        &mut self,
        epoch: &ExecutionEpoch,
        conversation: &str,
        id: &str,
        error: &AppError,
    ) -> Result<RunRecord> {
        let max = record_cap(self);
        let repository = self.evidence.repository_identity()?;
        let now = self.clock.now();
        let error = safe_failure(error);
        let completion = json(
            &RunCompletion::Failed {
                error: error.clone(),
            },
            ConversationLimits::TERMINAL_METADATA_BYTES,
        )?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        fence(&tx, &self.store_key, epoch)?;
        authorize(&tx, conversation, &repository)?;
        let mut run = run_read(&tx, conversation, id, max)?;
        if run.epoch != epoch.generation() {
            return Err(conflict());
        }
        if run.status == RunStatus::Failed {
            let same: bool = tx
                .query_row(
                    "SELECT completion=?2 FROM conversation_runs WHERE id=?1",
                    params![run.id, completion],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if !same {
                return Err(conflict());
            }
            tx.commit().map_err(storage)?;
            return Ok(run);
        }
        if run.status != RunStatus::Admitted {
            return Err(conflict());
        }
        run.status = RunStatus::Failed;
        run.error = Some(error);
        run.finished_at = Some(now);
        save_run(&tx, &run, max)?;
        tx.execute(
            "UPDATE conversation_runs SET completion=?2 WHERE id=?1",
            params![run.id, completion],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(run)
    }
    pub(super) fn conversation_finish(
        &mut self,
        attempt: &RunAttempt,
        completion: &RunCompletion,
    ) -> Result<RunRecord> {
        let max = record_cap(self);
        let repository = self.evidence.repository_identity()?;
        let now = self.clock.now();
        bound(
            completion,
            self.conversation_limits
                .assistant_bytes
                .max(ConversationLimits::TERMINAL_METADATA_BYTES),
        )?;
        let completion = match completion {
            RunCompletion::Failed { error } => RunCompletion::Failed {
                error: safe_failure(error),
            },
            other => other.clone(),
        };
        let completion_json = json(
            &completion,
            self.conversation_limits
                .assistant_bytes
                .max(ConversationLimits::TERMINAL_METADATA_BYTES),
        )?;
        let ids = &self.ids;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        fence(&tx, &self.store_key, &attempt.epoch)?;
        authorize(&tx, &attempt.conversation_id, &repository)?;
        let mut run = run_read(&tx, &attempt.conversation_id, &attempt.run_id, max)?;
        if run.epoch != attempt.epoch.generation() {
            return Err(conflict());
        }
        if run.status.is_terminal() {
            let same: bool = tx
                .query_row(
                    "SELECT completion=?2 FROM conversation_runs WHERE id=?1",
                    params![run.id, completion_json],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if !same {
                return Err(conflict());
            }
            tx.commit().map_err(storage)?;
            return Ok(run);
        }
        if run.status != RunStatus::Running {
            return Err(conflict());
        }
        let message = if let RunCompletion::Completed { text } = &completion {
            let message = Message {
                id: ids.next_id(),
                conversation_id: attempt.conversation_id.clone(),
                run_id: attempt.run_id.clone(),
                role: MessageRole::Assistant,
                text: text.clone(),
                created_at: now,
            };
            message.validate(&self.conversation_limits)?;
            Some((message.clone(), json(&message, max)?))
        } else {
            None
        };
        match completion {
            RunCompletion::Completed { .. } => run.status = RunStatus::Completed,
            RunCompletion::Failed { error } => {
                run.status = RunStatus::Failed;
                run.error = Some(error);
            }
            RunCompletion::Interrupted => run.status = RunStatus::Interrupted,
        }
        run.finished_at = Some(now);
        save_run(&tx, &run, max)?;
        if let Some((message, payload)) = message {
            tx.execute(
                "INSERT INTO conversation_messages VALUES(?1,?2,?3,?4)",
                params![message.id, message.conversation_id, message.run_id, payload],
            )
            .map_err(storage)?;
        }
        tx.execute(
            "UPDATE conversation_runs SET completion=?2 WHERE id=?1",
            params![run.id, completion_json],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(run)
    }
}
