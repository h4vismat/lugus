use super::*;
fn page<T: DeserializeOwned + Serialize>(
    tx: &Connection,
    table: &str,
    column: &str,
    key: &str,
    p: PageRequest,
    limits: &ConversationLimits,
    max: usize,
) -> Result<ConversationPage<T>> {
    let query =
        format!("SELECT payload FROM {table} WHERE {column}=?1 ORDER BY rowid LIMIT ?2 OFFSET ?3");
    let total_query = format!("SELECT count(*) FROM {table} WHERE {column}=?1");
    query_page(tx, &query, &total_query, key, p, limits, max)
}
fn query_page<T: DeserializeOwned + Serialize>(
    tx: &Connection,
    query: &str,
    total_query: &str,
    key: &str,
    p: PageRequest,
    limits: &ConversationLimits,
    max: usize,
) -> Result<ConversationPage<T>> {
    if p.limit == 0 || p.limit > limits.page_items || p.offset > i64::MAX as usize {
        return Err(limit());
    }
    let sizes =
        format!("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM ({query})");
    let (count, bytes): (i64, i64) = tx
        .query_row(&sizes, params![key, p.limit as i64, p.offset as i64], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(storage)?;
    check_size(bytes, max.min(limits.page_bytes))?;
    let total: i64 = tx
        .query_row(total_query, [key], |r| r.get(0))
        .map_err(storage)?;
    let end = p.offset.checked_add(count as usize).ok_or_else(limit)?;
    let next = ((end as u64) < total as u64).then_some(end);
    // Full page envelope preflight before loading any payloads; serialized records are canonical JSON.
    let overhead = serde_json::to_string(&ConversationPage::<u8> {
        items: vec![],
        next_offset: next,
    })
    .map_err(storage)?
    .len()
        + count.saturating_sub(1).max(0) as usize;
    if (bytes as usize).checked_add(overhead).ok_or_else(limit)? > max.min(limits.page_bytes) {
        return Err(limit());
    }
    let mut stmt = tx.prepare(query).map_err(storage)?;
    let items = stmt
        .query_map(params![key, p.limit as i64, p.offset as i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(storage)?
        .map(|v| serde_json::from_str(&v.map_err(storage)?).map_err(storage))
        .collect::<Result<Vec<T>>>()?;
    let result = ConversationPage {
        items,
        next_offset: next,
    };
    bound(&result, max.min(limits.page_bytes))?;
    Ok(result)
}
impl SqliteApplicationStore {
    pub(super) fn conversation_create(
        &mut self,
        request: &str,
        title: &str,
    ) -> Result<Conversation> {
        validate_id(request)?;
        if title.trim().is_empty() || title.len() > 1024 || title.chars().any(char::is_control) {
            return Err(error(
                ErrorKind::InvalidInput,
                "conversation title must be bounded control-free text",
            ));
        }
        let repository = self.evidence.repository_identity()?;
        let max = record_cap(self);
        {
            let tx = self.connection.unchecked_transaction().map_err(storage)?;
            let existing:Option<(bool,i64)>=tx.query_row("SELECT title=?3,length(CAST(id AS BLOB)) FROM conversations WHERE repository=?1 AND request=?2",params![repository,request,title],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
            if let Some((same, size)) = existing {
                if !same {
                    return Err(conflict());
                }
                check_size(size, Scope::MAX_ID_BYTES)?;
                let id: String = tx
                    .query_row(
                        "SELECT id FROM conversations WHERE repository=?1 AND request=?2",
                        params![repository, request],
                        |r| r.get(0),
                    )
                    .map_err(storage)?;
                let result = decode_one(
                    &tx,
                    "SELECT payload FROM conversations WHERE id=?1",
                    &id,
                    max,
                )?;
                tx.commit().map_err(storage)?;
                return Ok(result);
            }
            tx.commit().map_err(storage)?;
        }
        let c = Conversation {
            id: self.id()?,
            workspace_id: self.id()?,
            repository_id: repository,
            title: title.into(),
            created_at: self.clock.now(),
        };
        c.validate()?;
        let payload = json(&c, record_cap(self))?;
        let state = WorkspaceState {
            conversation_id: c.id.clone(),
            workspace_id: c.workspace_id.clone(),
            revision: 0,
            view_ids: vec![],
            selected_view_id: None,
        };
        let layout = json(&state, record_cap(self))?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        let existing:Option<(bool,i64)>=tx.query_row("SELECT title=?3,length(CAST(id AS BLOB)) FROM conversations WHERE repository=?1 AND request=?2",params![c.repository_id,request,title],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
        if let Some((same, size)) = existing {
            if !same {
                return Err(conflict());
            }
            check_size(size, Scope::MAX_ID_BYTES)?;
            let id: String = tx
                .query_row(
                    "SELECT id FROM conversations WHERE repository=?1 AND request=?2",
                    params![c.repository_id, request],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            let result = decode_one(
                &tx,
                "SELECT payload FROM conversations WHERE id=?1",
                &id,
                max,
            )?;
            tx.commit().map_err(storage)?;
            return Ok(result);
        }
        let collision:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM app_records WHERE workspace=?1 UNION ALL SELECT 1 FROM conversations WHERE workspace=?1 OR id=?2)",params![c.workspace_id,c.id],|r|r.get(0)).map_err(storage)?;
        if collision {
            return Err(conflict());
        }
        tx.execute("INSERT INTO conversations(id,workspace,repository,request,title,payload) VALUES(?1,?2,?3,?4,?5,?6)",params![c.id,c.workspace_id,c.repository_id,request,title,payload]).map_err(storage)?;
        tx.execute(
            "INSERT INTO conversation_workspaces VALUES(?1,?2,0,?3)",
            params![c.id, c.workspace_id, layout],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(c)
    }
    pub(super) fn conversation_read(&self, id: &str) -> Result<Conversation> {
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        authorize(&tx, id, &self.evidence.repository_identity()?)?;
        let c = decode_one(
            &tx,
            "SELECT payload FROM conversations WHERE id=?1",
            id,
            record_cap(self),
        )?;
        tx.commit().map_err(storage)?;
        Ok(c)
    }
    pub(super) fn conversation_list(
        &self,
        p: PageRequest,
    ) -> Result<ConversationPage<Conversation>> {
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let result = page(
            &tx,
            "conversations",
            "repository",
            &self.evidence.repository_identity()?,
            p,
            &self.conversation_limits,
            self.limits.max_output_bytes,
        )?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
    pub(super) fn conversation_recent_list(
        &self,
        p: PageRequest,
    ) -> Result<ConversationPage<Conversation>> {
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        // Recency is maintained on writes; this covering index walk never loads messages.
        let result = query_page(
            &tx,
            "SELECT c.payload FROM conversation_recency r
             JOIN conversations c ON c.id=r.conversation_id
             WHERE r.repository=?1 ORDER BY r.sequence DESC LIMIT ?2 OFFSET ?3",
            "SELECT count(*) FROM conversation_recency WHERE repository=?1",
            &self.evidence.repository_identity()?,
            p,
            &self.conversation_limits,
            self.limits.max_output_bytes,
        )?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
    pub(super) fn conversation_page<T: DeserializeOwned + Serialize>(
        &self,
        id: &str,
        run: Option<&str>,
        table: &str,
        column: &str,
        p: PageRequest,
    ) -> Result<ConversationPage<T>> {
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        authorize(&tx, id, &self.evidence.repository_identity()?)?;
        if let Some(run) = run {
            authorize_run(&tx, id, run)?;
        }
        let result = page(
            &tx,
            table,
            column,
            run.unwrap_or(id),
            p,
            &self.conversation_limits,
            self.limits.max_output_bytes,
        )?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
}
