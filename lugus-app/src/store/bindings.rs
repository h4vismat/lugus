//! Validate source evidence and create immutable company/instrument associations.
use super::binding_history::{append, conflict, dedupe, event};
use super::*;
use lugus_financial::instruments::InstrumentObservation;
use rusqlite::{OptionalExtension, params};
impl SqliteApplicationStore {
    pub(super) fn validate_binding_provenance(
        &self,
        p: &FetchProvenance,
        failed: bool,
    ) -> Result<()> {
        if let Some(observation) = &p.instrument_observation {
            let FetchCommand::InstrumentLookup { query, .. } = &p.command else {
                return Err(error(
                    ErrorKind::ScopeMismatch,
                    "instrument evidence requires lookup command",
                ));
            };
            let actual = self.evidence.instrument_observation(
                &p.provider,
                observation.id,
                self.read_limits(),
            )?;
            if failed
                || actual != *observation
                || actual.provider != p.provider
                || actual.request != *query
            {
                return Err(error(
                    ErrorKind::ScopeMismatch,
                    "instrument evidence ownership mismatch",
                ));
            }
        } else if matches!(p.command, FetchCommand::InstrumentLookup { .. }) && !failed {
            return Err(error(
                ErrorKind::MissingData,
                "successful lookup requires evidence",
            ));
        }
        if matches!(p.command, FetchCommand::InstrumentLookup { .. }) && p.document.is_some() {
            return Err(error(
                ErrorKind::ScopeMismatch,
                "lookup cannot contain document evidence",
            ));
        }
        if let Some(id) = &p.binding_id {
            // A trusted host may record work prepared before a subsequent state transition.
            let binding: BindingRecord =
                self.read(&p.scope, id, "binding", self.limits.max_output_bytes)?;
            let FetchCommand::Prices { query, .. } = &p.command else {
                return Err(error(
                    ErrorKind::ScopeMismatch,
                    "binding requires price command",
                ));
            };
            if binding.instrument.provider != p.provider
                || binding.instrument.request.instrument != query.instrument
            {
                return Err(error(
                    ErrorKind::ScopeMismatch,
                    "binding price association mismatch",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn create_binding(
        &mut self,
        scope: &Scope,
        request: &BindRequest,
    ) -> Result<BindingRecord> {
        scope.validate()?;
        sqlite::validate_id(&request.company_dataset_id)?;
        sqlite::validate_id(&request.instrument_fetch_id)?;
        if request.company_observation_id <= 0 {
            return Err(error(
                ErrorKind::InvalidInput,
                "invalid company observation",
            ));
        }
        if let Some(id) = &request.supersedes {
            sqlite::validate_id(id)?;
        }
        let input = json(
            &("bind", &scope.run_id, request),
            self.limits.max_input_bytes,
        )?;
        let repository = self.evidence.repository_identity()?;
        if let Some(id) = dedupe(
            &self.connection,
            scope,
            &repository,
            &input,
            self.limits.max_input_bytes,
        )? {
            return Ok(self.binding_view(scope, &id)?.record);
        }
        let header = self.dataset_header(scope, &request.company_dataset_id)?;
        if header.kind != DatasetKind::Resolution
            || header.error.is_some()
            || !matches!(
                header.resolution_status.as_deref(),
                Some("resolved" | "candidates")
            )
        {
            return Err(error(
                ErrorKind::Conflict,
                "resolution evidence is incomplete or conflicted",
            ));
        }
        let read_tx = self.connection.unchecked_transaction().map_err(storage)?;
        let size:Option<i64>=read_tx.query_row("SELECT length(CAST(payload AS BLOB)) FROM dataset_rows WHERE dataset_id=?1 AND observation_id=?2",params![header.id,request.company_observation_id],|r|r.get(0)).optional().map_err(storage)?;
        let size = size.ok_or_else(|| {
            error(
                ErrorKind::ScopeMismatch,
                "company observation is outside the dataset",
            )
        })?;
        if size < 0 || size as u64 > self.limits.max_read_page_bytes as u64 {
            return Err(limit());
        }
        let payload: String = read_tx
            .query_row(
                "SELECT payload FROM dataset_rows WHERE dataset_id=?1 AND observation_id=?2",
                params![header.id, request.company_observation_id],
                |r| r.get(0),
            )
            .map_err(storage)?;
        read_tx.commit().map_err(storage)?;
        let DatasetRow::Candidate { entry: company } =
            serde_json::from_str(&payload).map_err(storage)?
        else {
            return Err(error(
                ErrorKind::InvalidInput,
                "candidate evidence required",
            ));
        };
        let DatasetProjection::Resolution { run_id } = header.projection else {
            return Err(error(
                ErrorKind::InvalidInput,
                "resolution projection required",
            ));
        };
        if company.provider != header.provider
            || company.run_id != run_id
            || company.observation_id != request.company_observation_id
        {
            return Err(error(
                ErrorKind::ScopeMismatch,
                "company evidence ownership mismatch",
            ));
        }
        let fetch = self.read_fetch(scope, &request.instrument_fetch_id)?;
        let instrument = self.lookup_evidence(&fetch)?;
        let (policy, reasons) =
            match assess_binding(&company.candidate, &request.listing, &instrument.metadata) {
                BindingAssessment::Supported { policy, reasons } => (policy, reasons),
                BindingAssessment::Incomplete { .. } => {
                    return Err(error(
                        ErrorKind::MissingData,
                        "binding identity evidence is incomplete",
                    ));
                }
                BindingAssessment::Conflict { .. } => {
                    return Err(error(
                        ErrorKind::Conflict,
                        "binding identity evidence conflicts",
                    ));
                }
            };
        let previous = request
            .supersedes
            .as_ref()
            .map(|id| self.binding_view(scope, id))
            .transpose()?;
        if previous.as_ref().is_some_and(|b| {
            b.record.company.company != company.company
                || b.record.company.candidate.identifier != company.candidate.identifier
        }) {
            return Err(conflict());
        }
        let record = BindingRecord {
            id: self.id()?,
            scope: scope.clone(),
            repository_id: repository.clone(),
            company_dataset_id: header.id,
            company,
            listing: request.listing.clone(),
            instrument_fetch_id: fetch.id,
            instrument,
            policy,
            reasons,
            supersedes: request.supersedes.clone(),
            created_at: self.clock.now(),
        };
        let payload = json(&record, self.limits.max_output_bytes)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        if let Some(id) = dedupe(&tx, scope, &repository, &input, self.limits.max_input_bytes)? {
            tx.commit().map_err(storage)?;
            return Ok(self.binding_view(scope, &id)?.record);
        }
        if let Some(old) = &request.supersedes {
            if event(&tx, old, self.limits.max_output_bytes)?.status != BindingStatus::Active {
                return Err(conflict());
            }
            append(
                &tx,
                scope,
                old,
                BindingStatus::Superseded {
                    binding_id: record.id.clone(),
                },
                record.created_at,
                self.limits.max_output_bytes,
            )?;
        }
        tx.execute("INSERT INTO app_records(id,workspace,repository,category,payload) VALUES(?1,?2,?3,'binding',?4)",params![record.id,scope.workspace_id,repository,payload]).map_err(storage)?;
        tx.execute(
            "INSERT INTO binding_index(binding_id,workspace,repository) VALUES(?1,?2,?3)",
            params![record.id, scope.workspace_id, repository],
        )
        .map_err(storage)?;
        append(
            &tx,
            scope,
            &record.id,
            BindingStatus::Active,
            record.created_at,
            self.limits.max_output_bytes,
        )?;
        tx.execute("INSERT INTO binding_requests(workspace,repository,request,input,binding_id) VALUES(?1,?2,?3,?4,?5)",params![scope.workspace_id,repository,scope.request_id,input,record.id]).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(record)
    }
    fn lookup_evidence(&self, fetch: &FetchReference) -> Result<InstrumentObservation> {
        let FetchCommand::InstrumentLookup { query, .. } = &fetch.command else {
            return Err(error(
                ErrorKind::InvalidInput,
                "instrument lookup receipt required",
            ));
        };
        if fetch.error.is_some() || !fetch.runs.is_empty() {
            return Err(error(
                ErrorKind::MissingData,
                "successful lookup evidence required",
            ));
        }
        let saved = fetch
            .instrument_observation
            .as_ref()
            .ok_or_else(|| error(ErrorKind::MissingData, "lookup evidence missing"))?;
        let actual =
            self.evidence
                .instrument_observation(&fetch.provider, saved.id, self.read_limits())?;
        if &actual != saved || actual.provider != fetch.provider || actual.request != *query {
            return Err(error(
                ErrorKind::ScopeMismatch,
                "instrument evidence ownership mismatch",
            ));
        }
        Ok(actual)
    }
    pub(super) fn binding_view(&self, scope: &Scope, id: &str) -> Result<BindingView> {
        let record: BindingRecord =
            self.read(scope, id, "binding", self.limits.max_output_bytes)?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let status = event(&tx, id, self.limits.max_output_bytes)?.status;
        tx.commit().map_err(storage)?;
        let view = BindingView { record, status };
        json(&view, self.limits.max_output_bytes)?;
        Ok(view)
    }
}
