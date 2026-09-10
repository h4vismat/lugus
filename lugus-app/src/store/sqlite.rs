use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use std::path::Path;

pub struct SqliteApplicationStore {
    pub(super) connection: Connection,
    pub(super) evidence: Box<dyn EvidenceRepository>,
    pub(super) limits: Limits,
    pub(super) clock: Box<dyn Clock>,
    pub(super) ids: Box<dyn IdSource>,
}
impl SqliteApplicationStore {
    pub fn open(
        path: impl AsRef<Path>,
        evidence: Box<dyn EvidenceRepository>,
        limits: Limits,
        clock: Box<dyn Clock>,
        ids: Box<dyn IdSource>,
    ) -> Result<Self> {
        limits.validate()?;
        let mut connection = Connection::open(path).map_err(storage)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(storage)?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(storage)?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        let application: i64 = tx
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(storage)?;
        if version == 0 && application == 0 {
            let tables: i64 = tx
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if tables != 0 {
                return Err(error(
                    ErrorKind::Storage,
                    "application database must be separate",
                ));
            }
            tx.execute_batch("CREATE TABLE app_records(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,repository TEXT NOT NULL,category TEXT NOT NULL,payload TEXT NOT NULL); CREATE TABLE dataset_rows(dataset_id TEXT NOT NULL REFERENCES app_records(id),ordinal INTEGER NOT NULL,payload TEXT NOT NULL,observation_id INTEGER,PRIMARY KEY(dataset_id,ordinal)); CREATE TABLE view_requests(workspace TEXT NOT NULL,request TEXT NOT NULL,input TEXT NOT NULL,view_id TEXT NOT NULL REFERENCES app_records(id),PRIMARY KEY(workspace,request)); PRAGMA application_id=1280657235; PRAGMA user_version=1;").map_err(storage)?;
        } else if !(1..=2).contains(&version) || application != 1280657235 {
            return Err(error(
                ErrorKind::Storage,
                "unsupported application database schema",
            ));
        }
        if version < 2 {
            super::binding_history::migrate(&tx)?;
        }
        tx.commit().map_err(storage)?;
        Ok(Self {
            connection,
            evidence,
            limits,
            clock,
            ids,
        })
    }
    pub(super) fn id(&self) -> Result<String> {
        let id = self.ids.next_id();
        validate_id(&id)?;
        Ok(id)
    }
    pub(super) fn read<T: DeserializeOwned>(
        &self,
        scope: &Scope,
        id: &str,
        category: &str,
        max: usize,
    ) -> Result<T> {
        scope.validate()?;
        validate_id(id)?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        // Scope/repository comparison remains in SQLite: even corrupt metadata is not loaded.
        let repository = self.evidence.repository_identity()?;
        let metadata:Option<(bool,bool,bool,i64)>=tx.query_row("SELECT workspace=?2,repository=?3,category=?4,length(CAST(payload AS BLOB)) FROM app_records WHERE id=?1",params![id,scope.workspace_id,repository,category],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(storage)?;
        let (workspace, repository, kind, size) =
            metadata.ok_or_else(|| error(ErrorKind::MissingData, "reference does not exist"))?;
        if !workspace || !repository {
            return Err(error(
                ErrorKind::ScopeMismatch,
                "reference belongs to another workspace or repository",
            ));
        }
        if !kind {
            return Err(error(
                ErrorKind::InvalidInput,
                "reference kind is incompatible",
            ));
        }
        if size < 0 || size as u64 > max as u64 {
            return Err(limit());
        }
        let payload: String = tx
            .query_row("SELECT payload FROM app_records WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(storage)?;
        let result = serde_json::from_str(&payload).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
    pub(super) fn insert(
        &self,
        id: &str,
        scope: &str,
        repository: &str,
        category: &str,
        payload: &str,
    ) -> Result<()> {
        self.connection.execute("INSERT INTO app_records(id,workspace,repository,category,payload) VALUES(?1,?2,?3,?4,?5)",params![id,scope,repository,category,payload]).map_err(storage)?;
        Ok(())
    }
}
pub(super) fn validate_id(id: &str) -> Result<()> {
    if id.trim().is_empty() || id.len() > Scope::MAX_ID_BYTES || id.chars().any(char::is_control) {
        Err(error(
            ErrorKind::InvalidInput,
            "invalid reference identifier",
        ))
    } else {
        Ok(())
    }
}
impl ApplicationStore for SqliteApplicationStore {
    fn bind(&mut self, s: &Scope, r: &BindRequest) -> Result<BindingRecord> {
        self.create_binding(s, r)
    }
    fn read_binding(&self, s: &Scope, id: &str) -> Result<BindingView> {
        self.binding_view(s, id)
    }
    fn list_bindings(&self, s: &Scope, p: PageRequest) -> Result<BindingPage> {
        self.binding_page(s, p)
    }
    fn revoke_binding(&mut self, s: &Scope, r: &RevokeBindingRequest) -> Result<BindingView> {
        self.revoke(s, r)
    }
    fn prepare_binding(&self, s: &Scope, id: &str) -> Result<BindingRecord> {
        let view = self.binding_state(s, id)?;
        if view.status != BindingStatus::Active {
            return Err(error(ErrorKind::Conflict, "binding is no longer active"));
        }
        Ok(view.record)
    }
    fn binding_history(&self, s: &Scope, id: &str, p: PageRequest) -> Result<BindingHistoryPage> {
        self.history(s, id, p)
    }

    fn record_fetch(&mut self, result: &FetchResult) -> Result<FetchReference> {
        let p = &result.provenance;
        p.scope.validate()?;
        p.command.validate()?;
        json(p, self.limits.max_input_bytes)?;
        if p.repository_id != self.evidence.repository_identity()?
            || p.command.instance_id() != p.provider.instance_id
            || p.document
                .as_ref()
                .is_some_and(|d| d.provider != p.provider)
        {
            return Err(error(
                ErrorKind::ScopeMismatch,
                "fetch provenance scope mismatch",
            ));
        }
        // Runs are ingestion attempts, not provider pages. Resolve can start a
        // fallback after its primary attempt consumed the entire page budget.
        let (run_kind, max_runs) = match p.command.operation() {
            Operation::Resolve => (Some(RunKind::Resolution), 2),
            Operation::Lookup => (Some(RunKind::Resolution), 1),
            Operation::Filings | Operation::Facts => (Some(RunKind::Financial), 1),
            Operation::Prices => (Some(RunKind::Market), 1),
            Operation::Document | Operation::InstrumentLookup => (None, 0),
        };
        if p.runs.len() > max_runs || p.runs.iter().any(|r| r.id <= 0 || Some(r.kind) != run_kind) {
            return Err(error(ErrorKind::InvalidInput, "invalid fetch run receipts"));
        }
        self.validate_binding_provenance(p, result.error.is_some())?;
        let reference = FetchReference {
            id: self.id()?,
            scope: p.scope.clone(),
            provider: p.provider.clone(),
            repository_id: p.repository_id.clone(),
            command: p.command.clone(),
            runs: p.runs.clone(),
            document: p.document.clone(),
            instrument_observation: p.instrument_observation.clone(),
            binding_id: p.binding_id.clone(),
            error: safe_error(&result.error),
            created_at: self.clock.now(),
        };
        let payload = json(&reference, self.limits.max_input_bytes)?;
        self.insert(
            &reference.id,
            &p.scope.workspace_id,
            &p.repository_id,
            "fetch",
            &payload,
        )?;
        Ok(reference)
    }
    fn read_fetch(&self, scope: &Scope, id: &str) -> Result<FetchReference> {
        self.read(scope, id, "fetch", self.limits.max_input_bytes)
    }
    fn create_dataset(
        &mut self,
        scope: &Scope,
        id: &str,
        projection: DatasetProjection,
    ) -> Result<DatasetHeader> {
        self.freeze(scope, id, projection)
    }
    fn dataset_header(&self, scope: &Scope, id: &str) -> Result<DatasetHeader> {
        self.read(scope, id, "dataset", self.limits.max_output_bytes)
    }
    fn read_dataset(&self, scope: &Scope, id: &str, page: PageRequest) -> Result<DatasetPage> {
        if page.limit == 0
            || page.limit > self.limits.max_read_page_items
            || page.offset > i64::MAX as usize
        {
            return Err(limit());
        }
        let header = self.dataset_header(scope, id)?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let bytes:i64=tx.query_row("SELECT coalesce(sum(size),0) FROM (SELECT length(CAST(payload AS BLOB)) size FROM dataset_rows WHERE dataset_id=?1 AND ordinal>=?2 ORDER BY ordinal LIMIT ?3)",params![id,page.offset as i64,page.limit as i64],|r|r.get(0)).map_err(storage)?;
        if bytes < 0 || bytes as u64 > self.limits.max_read_page_bytes as u64 {
            return Err(limit());
        }
        let rows = {
            let mut stmt=tx.prepare("SELECT payload FROM dataset_rows WHERE dataset_id=?1 AND ordinal>=?2 ORDER BY ordinal LIMIT ?3").map_err(storage)?;
            stmt.query_map(params![id, page.offset as i64, page.limit as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(storage)?
            .map(|r| serde_json::from_str(&r.map_err(storage)?).map_err(storage))
            .collect::<Result<Vec<DatasetRow>>>()?
        };
        tx.commit().map_err(storage)?;
        let end = page.offset.checked_add(rows.len()).ok_or_else(limit)?;
        let result = DatasetPage {
            next_offset: (end < header.row_count).then_some(end),
            header,
            rows,
        };
        json(&result, self.limits.max_output_bytes)?;
        Ok(result)
    }
    fn read_document(
        &self,
        scope: &Scope,
        id: &str,
        offset: usize,
        length: usize,
    ) -> Result<DocumentRead> {
        if length == 0 || length > self.limits.max_read_page_bytes {
            return Err(limit());
        }
        let h = self.dataset_header(scope, id)?;
        let observation = h
            .document
            .ok_or_else(|| error(ErrorKind::InvalidInput, "reference is not a document"))?;
        let content = self
            .evidence
            .document(&observation, self.limits.max_document_bytes)?;
        if offset > content.len() {
            return Err(error(
                ErrorKind::InvalidInput,
                "document offset exceeds original byte length",
            ));
        }
        let end = offset
            .checked_add(length)
            .ok_or_else(limit)?
            .min(content.len());
        let result = DocumentRead {
            dataset_id: id.into(),
            observation,
            offset,
            total_bytes: content.len(),
            bytes: content[offset..end].to_vec(),
        };
        json(&result, self.limits.max_output_bytes)?;
        Ok(result)
    }
    fn select_candidate(
        &mut self,
        scope: &Scope,
        id: &str,
        observation_id: i64,
    ) -> Result<lugus_financial::resolution::catalog::CatalogSelection> {
        let h = self.dataset_header(scope, id)?;
        let DatasetProjection::Resolution { run_id } = h.projection else {
            return Err(error(
                ErrorKind::InvalidInput,
                "candidate reference required",
            ));
        };
        let member:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM dataset_rows WHERE dataset_id=?1 AND observation_id=?2)",params![id,observation_id],|r|r.get(0)).map_err(storage)?;
        if !member {
            return Err(error(
                ErrorKind::ScopeMismatch,
                "candidate is outside the authorized reference",
            ));
        }
        let mut selection = self.evidence.select_candidate(
            &h.provider,
            run_id,
            observation_id,
            self.read_limits(),
        )?;
        selection.error = selection
            .error
            .map(|_| "resolution did not complete successfully".into());
        if let Some(f) = &mut selection.failure {
            f.message = "resolution did not complete successfully".into();
        }
        json(&selection, self.limits.max_output_bytes)?;
        Ok(selection)
    }
    fn open_view(&mut self, scope: &Scope, request: &OpenViewRequest) -> Result<ViewReceipt> {
        self.accept_view(scope, request)
    }
    fn read_view(&self, scope: &Scope, id: &str) -> Result<ViewReceipt> {
        self.read(scope, id, "view", self.limits.max_output_bytes)
    }
    fn report_presentation(&mut self, scope: &Scope, result: &PresentationResult) -> Result<()> {
        self.presentation(scope, result)
    }
}
