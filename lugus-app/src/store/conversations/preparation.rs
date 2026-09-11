use super::*;

fn preparation_cap(store: &SqliteApplicationStore) -> usize {
    // A returned String may encode every byte as a six-byte JSON escape, plus quotes.
    store
        .conversation_limits
        .selected_bytes
        .min(store.conversation_limits.context_bytes)
        .min(store.limits.max_output_bytes.saturating_sub(2) / 6)
}

impl SqliteApplicationStore {
    pub(super) fn preparation_save(
        &mut self,
        attempt: &RunAttempt,
        serialized: &str,
    ) -> Result<()> {
        let cap = preparation_cap(self);
        check_size(serialized.len() as i64, cap)?;
        let value: serde_json::Value = serde_json::from_str(serialized)
            .map_err(|_| error(ErrorKind::InvalidInput, "preparation must be a JSON object"))?;
        if !value.is_object() {
            return Err(error(
                ErrorKind::InvalidInput,
                "preparation must be a JSON object",
            ));
        }
        let repository = self.evidence.repository_identity()?;
        let record_max = record_cap(self);
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        active(&tx, &self.store_key, attempt, &repository, record_max)?;
        // Compare in SQLite so an existing corrupt oversized receipt is never materialized.
        let same: Option<bool> = tx
            .query_row(
                "SELECT payload=?2 FROM conversation_preparations WHERE run_id=?1",
                params![attempt.run_id, serialized],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        match same {
            Some(true) => (),
            Some(false) => return Err(conflict()),
            None => {
                tx.execute(
                    "INSERT INTO conversation_preparations(run_id,payload) VALUES(?1,?2)",
                    params![attempt.run_id, serialized],
                )
                .map_err(storage)?;
            }
        }
        tx.commit().map_err(storage)
    }

    pub(super) fn preparation_read(
        &self,
        conversation: &str,
        run: Option<&str>,
    ) -> Result<Option<String>> {
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        authorize(&tx, conversation, &self.evidence.repository_identity()?)?;
        if let Some(run) = run {
            authorize_run(&tx, conversation, run)?;
        }
        let sql = "SELECT p.payload FROM conversation_preparations p
            JOIN conversation_runs r ON r.id=p.run_id
            WHERE r.conversation_id=?1 AND (?2 IS NULL OR r.id=?2)
            ORDER BY r.rowid DESC LIMIT 1";
        let metadata = format!("SELECT length(CAST(payload AS BLOB)) FROM ({sql})");
        let size: Option<i64> = tx
            .query_row(&metadata, params![conversation, run], |r| r.get(0))
            .optional()
            .map_err(storage)?;
        let result = match size {
            None => None,
            Some(size) => {
                check_size(size, preparation_cap(self))?;
                Some(
                    tx.query_row(sql, params![conversation, run], |r| r.get(0))
                        .map_err(storage)?,
                )
            }
        };
        tx.commit().map_err(storage)?;
        Ok(result)
    }
}
