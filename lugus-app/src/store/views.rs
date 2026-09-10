use super::*;
use rusqlite::{OptionalExtension, params};
impl SqliteApplicationStore {
    pub(super) fn accept_view(
        &mut self,
        scope: &Scope,
        request: &OpenViewRequest,
    ) -> Result<ViewReceipt> {
        scope.validate()?;
        let header = self.dataset_header(scope, &request.dataset_id)?;
        let input = json(request, self.limits.max_input_bytes)?;
        let id = self.id()?;
        let accepted_at = self.clock.now();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        let existing:Option<(bool,i64,i64)>=tx.query_row("SELECT v.input=?3,length(CAST(r.payload AS BLOB)),length(CAST(v.view_id AS BLOB)) FROM view_requests v JOIN app_records r ON r.id=v.view_id WHERE v.workspace=?1 AND v.request=?2",params![scope.workspace_id,scope.request_id,input],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)?;
        if let Some((same, size, id_size)) = existing {
            if !same {
                return Err(error(
                    ErrorKind::Conflict,
                    "request identifier already accepted with different input",
                ));
            }
            if size < 0
                || size as u64 > self.limits.max_output_bytes as u64
                || id_size < 0
                || id_size as u64 > Scope::MAX_ID_BYTES as u64
            {
                return Err(limit());
            }
            let payload:String=tx.query_row("SELECT r.payload FROM view_requests v JOIN app_records r ON r.id=v.view_id WHERE v.workspace=?1 AND v.request=?2",params![scope.workspace_id,scope.request_id],|r|r.get(0)).map_err(storage)?;
            let receipt = serde_json::from_str(&payload).map_err(storage)?;
            tx.commit().map_err(storage)?;
            return Ok(receipt);
        }
        let compatible = match request.kind {
            ViewKind::PriceChart => header.kind == DatasetKind::Prices,
            ViewKind::DataTable => header.kind != DatasetKind::Document,
            ViewKind::Document => header.kind == DatasetKind::Document,
        };
        if !compatible {
            return Err(error(
                ErrorKind::Unsupported,
                "view kind is incompatible with dataset",
            ));
        }
        let receipt = ViewReceipt {
            id,
            workspace_id: scope.workspace_id.clone(),
            request_id: scope.request_id.clone(),
            dataset_id: request.dataset_id.clone(),
            kind: request.kind,
            descriptor_revision: 1,
            accepted_at,
            presentation: None,
        };
        let payload = json(&receipt, self.limits.max_output_bytes)?;
        tx.execute("INSERT INTO app_records(id,workspace,repository,category,payload) VALUES(?1,?2,?3,'view',?4)",params![receipt.id,scope.workspace_id,header.repository_id,payload]).map_err(storage)?;
        tx.execute(
            "INSERT INTO view_requests(workspace,request,input,view_id) VALUES(?1,?2,?3,?4)",
            params![scope.workspace_id, scope.request_id, input, receipt.id],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(receipt)
    }
    pub(super) fn presentation(
        &mut self,
        scope: &Scope,
        result: &PresentationResult,
    ) -> Result<()> {
        let mut receipt = self.read_view(scope, &result.view_id)?;
        if receipt.descriptor_revision != result.descriptor_revision {
            return Err(error(
                ErrorKind::StaleReference,
                "presentation descriptor revision is stale",
            ));
        }
        receipt.presentation = Some(result.status);
        let payload = json(&receipt, self.limits.max_output_bytes)?;
        self.connection
            .execute(
                "UPDATE app_records SET payload=?2 WHERE id=?1",
                params![result.view_id, payload],
            )
            .map_err(storage)?;
        Ok(())
    }
}
