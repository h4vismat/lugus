use super::*;
use lugus_financial::{
    resolution::catalog::ResolutionOutcome,
    selection::{SelectionManifest, select_daily, select_facts},
    storage::{RunStatus, bounded::ReadLimits},
};
use rusqlite::params;
impl SqliteApplicationStore {
    pub(super) fn read_limits(&self) -> ReadLimits {
        ReadLimits {
            max_items: self.limits.max_items_per_fetch,
            max_bytes: self.limits.max_bytes_per_fetch,
        }
    }
    pub(super) fn freeze(
        &mut self,
        scope: &Scope,
        id: &str,
        projection: DatasetProjection,
    ) -> Result<DatasetHeader> {
        let receipt = self.read_fetch(scope, id)?;
        json(&projection, self.limits.max_input_bytes)?;
        let mut header = DatasetHeader {
            binding_id: receipt.binding_id.clone(),
            id: self.id()?,
            workspace_id: scope.workspace_id.clone(),
            repository_id: receipt.repository_id.clone(),
            provider: receipt.provider.clone(),
            fetch_id: id.into(),
            kind: DatasetKind::Document,
            projection: projection.clone(),
            query: serde_json::to_value(&receipt.command).map_err(storage)?,
            created_at: self.clock.now(),
            row_count: 0,
            policy: None,
            selected_run: None,
            coverage: None,
            limitations: vec![],
            conflicts: vec![],
            resolution_status: None,
            source_snapshot: None,
            source_coverage: None,
            document: None,
            error: receipt.error.clone(),
        };
        let authorize = |kind, id| {
            if receipt.runs.iter().any(|r| r.kind == kind && r.id == id) {
                Ok(())
            } else {
                Err(error(
                    ErrorKind::ScopeMismatch,
                    "run is outside the authorized fetch reference",
                ))
            }
        };
        let rows = match projection {
            DatasetProjection::Prices {
                run_id,
                query,
                series,
            } => {
                authorize(RunKind::Market, run_id)?;
                let evidence =
                    self.evidence
                        .market_run(&receipt.provider, run_id, self.read_limits())?;
                terminal(evidence.run.status)?;
                let selected =
                    select_daily(&receipt.provider, &query, series, &[evidence], Some(run_id))
                        .map_err(evidence::financial)?;
                header.kind = DatasetKind::Prices;
                manifest(&mut header, selected.manifest);
                header.coverage = selected.coverage;
                header.conflicts = selected.conflicts;
                selected
                    .prices
                    .into_iter()
                    .enumerate()
                    .map(|(i, evidence)| DatasetRow::Price {
                        evidence,
                        value: selected.values.get(i).cloned().flatten(),
                    })
                    .collect::<Vec<_>>()
            }
            DatasetProjection::Facts { run_id, query } => {
                authorize(RunKind::Financial, run_id)?;
                let evidence =
                    self.evidence
                        .financial_run(&receipt.provider, run_id, self.read_limits())?;
                terminal(evidence.run.status)?;
                let selected = select_facts(&receipt.provider, &query, &[evidence], Some(run_id))
                    .map_err(evidence::financial)?;
                header.kind = DatasetKind::Facts;
                manifest(&mut header, selected.manifest);
                selected
                    .groups
                    .into_iter()
                    .map(|group| DatasetRow::Fact { group })
                    .collect()
            }
            DatasetProjection::Filings { run_id } => {
                authorize(RunKind::Financial, run_id)?;
                let evidence =
                    self.evidence
                        .financial_run(&receipt.provider, run_id, self.read_limits())?;
                terminal(evidence.run.status)?;
                if evidence.run.operation != "filings" && evidence.run.operation != "both" {
                    return Err(error(
                        ErrorKind::InvalidInput,
                        "run does not contain filings",
                    ));
                }
                header.kind = DatasetKind::Filings;
                header.query = serde_json::to_value(&evidence.run.query).map_err(storage)?;
                header.selected_run = Some(lugus_financial::selection::RunReference {
                    context: evidence.context,
                    kind: "financial".into(),
                    run_id,
                    status: evidence.run.status,
                    error: evidence
                        .run
                        .error
                        .map(|_| "ingestion did not complete successfully".into()),
                });
                evidence
                    .filings
                    .into_iter()
                    .map(|evidence| DatasetRow::Filing { evidence })
                    .collect()
            }
            DatasetProjection::Resolution { run_id } => {
                authorize(RunKind::Resolution, run_id)?;
                header.kind = DatasetKind::Resolution;
                let evidence =
                    self.evidence
                        .resolution_run(&receipt.provider, run_id, self.read_limits())?;
                header.query = serde_json::to_value(&evidence.request).map_err(storage)?;
                let outcome = evidence.outcome;
                let (status, _items, snapshot, coverage) = match outcome {
                    ResolutionOutcome::Resolved {
                        entry,
                        snapshot,
                        coverage,
                    } => ("resolved", vec![*entry], Some(snapshot), Some(coverage)),
                    ResolutionOutcome::Candidates {
                        items,
                        snapshot,
                        coverage,
                    } => ("candidates", items, Some(snapshot), Some(coverage)),
                    ResolutionOutcome::NoMatch { snapshot, coverage } => {
                        ("no_match", vec![], Some(snapshot), Some(coverage))
                    }
                    ResolutionOutcome::Incomplete {
                        items,
                        snapshot,
                        coverage,
                        ..
                    } => ("incomplete", items, snapshot, coverage),
                    ResolutionOutcome::IdentityConflict {
                        items,
                        snapshot,
                        coverage,
                    } => {
                        header
                            .conflicts
                            .push("Stored catalog contains conflicting identities".into());
                        ("identity_conflict", items, Some(snapshot), Some(coverage))
                    }
                };
                header.resolution_status = Some(status.into());
                header.source_snapshot = snapshot;
                header.source_coverage = coverage;
                // Historical conflict evidence is explanatory, never authorization to select a
                // candidate retrieved by another run/provider.
                evidence
                    .entries
                    .into_iter()
                    .filter(|e| e.run_id == run_id && e.provider == receipt.provider)
                    .map(|entry| DatasetRow::Candidate { entry })
                    .collect()
            }
            DatasetProjection::Document => {
                let document = receipt.document.ok_or_else(|| {
                    error(
                        ErrorKind::MissingData,
                        "fetch contains no original document",
                    )
                })?;
                let FetchCommand::Document { source_url, .. } = &receipt.command else {
                    return Err(error(ErrorKind::ScopeMismatch, "document command required"));
                };
                if document.source_url != *source_url || document.provider != receipt.provider {
                    return Err(error(
                        ErrorKind::ScopeMismatch,
                        "document source association mismatch",
                    ));
                }
                self.evidence
                    .document(&document, self.limits.max_document_bytes)?;
                header.document = Some(document);
                vec![]
            }
        };
        header.row_count = rows.len();
        if rows.len() > self.limits.max_items_per_fetch {
            return Err(limit());
        }
        let payload = json(&header, self.limits.max_output_bytes)?;
        let mut bytes = payload.len();
        if bytes > self.limits.max_bytes_per_fetch {
            return Err(limit());
        }
        let mut encoded = Vec::with_capacity(rows.len());
        for row in rows {
            let observation = match &row {
                DatasetRow::Candidate { entry } => Some(entry.observation_id),
                _ => None,
            };
            let value = json(&row, self.limits.max_read_page_bytes)?;
            bytes = bytes.checked_add(value.len()).ok_or_else(limit)?;
            if bytes > self.limits.max_bytes_per_fetch {
                return Err(limit());
            }
            encoded.push((value, observation));
        }
        let tx = self.connection.transaction().map_err(storage)?;
        tx.execute("INSERT INTO app_records(id,workspace,repository,category,payload) VALUES(?1,?2,?3,'dataset',?4)",params![header.id,header.workspace_id,header.repository_id,payload]).map_err(storage)?;
        for (i, (value, observation)) in encoded.into_iter().enumerate() {
            tx.execute("INSERT INTO dataset_rows(dataset_id,ordinal,payload,observation_id) VALUES(?1,?2,?3,?4)",params![header.id,i as i64,value,observation]).map_err(storage)?;
        }
        tx.commit().map_err(storage)?;
        Ok(header)
    }
}
fn terminal(status: RunStatus) -> Result<()> {
    if status == RunStatus::Running {
        Err(error(
            ErrorKind::Conflict,
            "running evidence cannot be frozen",
        ))
    } else {
        Ok(())
    }
}
fn manifest(h: &mut DatasetHeader, m: SelectionManifest) {
    h.query = m.query;
    h.policy = Some(m.policy);
    h.selected_run = m.selected_run.map(|mut r| {
        r.error = r
            .error
            .map(|_| "ingestion did not complete successfully".into());
        r
    });
    h.coverage = m.source_coverage;
    h.limitations = m.limitations;
}
