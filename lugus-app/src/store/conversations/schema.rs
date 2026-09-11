use super::*;
pub(crate) fn migrate(tx: &Connection, limits: &ConversationLimits) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE conversation_config (
            singleton INTEGER PRIMARY KEY CHECK(singleton=1),
            limits TEXT NOT NULL,
            epoch INTEGER NOT NULL
        );
        CREATE TABLE conversations (
            id TEXT PRIMARY KEY,
            workspace TEXT NOT NULL UNIQUE,
            repository TEXT NOT NULL,
            request TEXT NOT NULL,
            title TEXT NOT NULL,
            payload TEXT NOT NULL,
            UNIQUE(repository,request)
        );
        CREATE TABLE conversation_workspaces (
            conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
            workspace TEXT NOT NULL UNIQUE,
            revision INTEGER NOT NULL,
            payload TEXT NOT NULL
        );
        CREATE TABLE conversation_runs (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL REFERENCES conversations(id),
            request TEXT NOT NULL,
            request_input TEXT NOT NULL,
            input TEXT NOT NULL,
            epoch INTEGER NOT NULL,
            status TEXT NOT NULL,
            payload TEXT NOT NULL,
            completion TEXT,
            UNIQUE(conversation_id,request)
        );
        CREATE UNIQUE INDEX conversation_active ON conversation_runs(conversation_id)
            WHERE status IN ('admitted','running');
        CREATE INDEX conversation_run_order ON conversation_runs(conversation_id);
        CREATE TABLE conversation_messages (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL REFERENCES conversations(id),
            run_id TEXT NOT NULL REFERENCES conversation_runs(id),
            payload TEXT NOT NULL
        );
        CREATE INDEX conversation_message_order ON conversation_messages(conversation_id);
        CREATE INDEX conversation_message_run ON conversation_messages(run_id);
        CREATE TABLE conversation_activity (
            run_id TEXT NOT NULL REFERENCES conversation_runs(id),
            sequence INTEGER NOT NULL,
            payload TEXT NOT NULL,
            PRIMARY KEY(run_id,sequence)
        );
        CREATE TABLE conversation_tools (
            run_id TEXT NOT NULL REFERENCES conversation_runs(id),
            call_id TEXT NOT NULL,
            intent TEXT NOT NULL,
            payload TEXT NOT NULL,
            reserved INTEGER NOT NULL,
            finished INTEGER NOT NULL,
            PRIMARY KEY(run_id,call_id)
        );
        PRAGMA user_version=3;",
    )
    .map_err(storage)?;
    for table in ["conversation_messages", "conversation_activity"] {
        for operation in ["UPDATE", "DELETE"] {
            tx.execute_batch(&format!(
                "CREATE TRIGGER {table}_{operation} BEFORE {operation} ON {table}
                BEGIN
                    SELECT RAISE(ABORT,'immutable conversation record');
                END;"
            ))
            .map_err(storage)?;
        }
    }
    tx.execute_batch(
        "CREATE TRIGGER conversation_run_input_immutable
            BEFORE UPDATE OF id,conversation_id,request,request_input,input,epoch
            ON conversation_runs
        BEGIN
            SELECT RAISE(ABORT,'immutable run input');
        END;
        CREATE TRIGGER conversation_run_delete BEFORE DELETE ON conversation_runs
        BEGIN
            SELECT RAISE(ABORT,'immutable run');
        END;
        CREATE TRIGGER conversation_tool_intent_immutable
            BEFORE UPDATE OF run_id,call_id,intent,reserved ON conversation_tools
        BEGIN
            SELECT RAISE(ABORT,'immutable tool intent');
        END;
        CREATE TRIGGER conversation_tool_result_immutable BEFORE UPDATE ON conversation_tools
            WHEN OLD.finished=1
        BEGIN
            SELECT RAISE(ABORT,'immutable tool result');
        END;
        CREATE TRIGGER conversation_tool_delete BEFORE DELETE ON conversation_tools
        BEGIN
            SELECT RAISE(ABORT,'immutable tool journal');
        END;",
    )
    .map_err(storage)?;
    tx.execute(
        "INSERT INTO conversation_config VALUES(1,?1,0)",
        [json(limits, ConversationLimits::TERMINAL_METADATA_BYTES)?],
    )
    .map_err(storage)?;
    Ok(())
}
pub(crate) fn configured_limits(tx: &Connection) -> Result<ConversationLimits> {
    let size: i64 = tx
        .query_row(
            "SELECT length(CAST(limits AS BLOB)) FROM conversation_config WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(storage)?;
    check_size(size, ConversationLimits::TERMINAL_METADATA_BYTES)?;
    let value: String = tx
        .query_row(
            "SELECT limits FROM conversation_config WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(storage)?;
    serde_json::from_str(&value).map_err(storage)
}

/// One-time backfill reads only the last message per chat. Subsequent ordering uses a
/// transactional sequence, independent of clock precision or clock adjustments.
pub(crate) fn migrate_recency(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE conversation_recency (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id TEXT NOT NULL UNIQUE REFERENCES conversations(id),
            repository TEXT NOT NULL
        );
        CREATE INDEX conversation_recency_order ON conversation_recency(repository,sequence);
        INSERT INTO conversation_recency(conversation_id,repository)
            SELECT c.id,c.repository FROM conversations c
            LEFT JOIN conversation_messages m ON m.rowid=(
                SELECT rowid FROM conversation_messages
                WHERE conversation_id=c.id ORDER BY rowid DESC LIMIT 1
            )
            ORDER BY julianday(coalesce(json_extract(m.payload,'$.created_at'),
                                        json_extract(c.payload,'$.created_at'))),
                     coalesce(m.rowid,0),c.rowid;
        CREATE TRIGGER conversation_recency_create AFTER INSERT ON conversations
        BEGIN
            INSERT INTO conversation_recency(conversation_id,repository)
            VALUES(NEW.id,NEW.repository);
        END;
        CREATE TRIGGER conversation_recency_message AFTER INSERT ON conversation_messages
        BEGIN
            DELETE FROM conversation_recency WHERE conversation_id=NEW.conversation_id;
            INSERT INTO conversation_recency(conversation_id,repository)
                SELECT id,repository FROM conversations WHERE id=NEW.conversation_id;
        END;
        PRAGMA user_version=5;",
    )
    .map_err(storage)
}

/// Add immutable preparation receipts without rewriting any existing run input.
pub(crate) fn migrate_preparations(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE conversation_preparations (
            run_id TEXT PRIMARY KEY REFERENCES conversation_runs(id),
            payload TEXT NOT NULL
        );
        CREATE TRIGGER conversation_preparations_update BEFORE UPDATE ON conversation_preparations
        BEGIN SELECT RAISE(ABORT,'immutable research preparation'); END;
        CREATE TRIGGER conversation_preparations_delete BEFORE DELETE ON conversation_preparations
        BEGIN SELECT RAISE(ABORT,'immutable research preparation'); END;
        PRAGMA user_version=6;",
    )
    .map_err(storage)
}
