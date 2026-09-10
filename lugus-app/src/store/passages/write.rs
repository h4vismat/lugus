use super::*;
fn cached(
    db: &Connection,
    scope: &Scope,
    repo: &str,
    dataset: &str,
    extractor: &str,
    max: usize,
) -> Result<Option<TextRepresentation>> {
    let size:Option<i64>=db.query_row("SELECT length(CAST(id AS BLOB)) FROM text_representations WHERE workspace=?1 AND repository=?2 AND dataset=?3 AND extractor=?4",params![scope.workspace_id,repo,dataset,extractor],|r|r.get(0)).optional().map_err(storage)?;
    let Some(size) = size else { return Ok(None) };
    bounded(size, Scope::MAX_ID_BYTES)?;
    let id:String=db.query_row("SELECT id FROM text_representations WHERE workspace=?1 AND repository=?2 AND dataset=?3 AND extractor=?4",params![scope.workspace_id,repo,dataset,extractor],|r|r.get(0)).map_err(storage)?;
    let h = read::header(db, scope, repo, &id, max)?;
    if h.dataset_id != dataset || json(&h.extractor, HEADER_MAX)? != extractor {
        return Err(corrupt());
    }
    Ok(Some(h))
}
fn duplicate_passage(
    db: &Connection,
    scope: &Scope,
    repo: &str,
    input: &str,
    max: usize,
) -> Result<Option<Passage>> {
    let existing: Option<(bool,i64)> = db.query_row("SELECT input=?3,length(CAST(passage AS BLOB)) FROM passage_requests WHERE workspace=?1 AND request=?2", params![scope.workspace_id,scope.request_id,input], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
    let Some((same, size)) = existing else {
        return Ok(None);
    };
    if !same {
        return Err(error(
            ErrorKind::Conflict,
            "passage request already accepted with different input",
        ));
    }
    bounded(size, Scope::MAX_ID_BYTES)?;
    let id: String = db
        .query_row(
            "SELECT passage FROM passage_requests WHERE workspace=?1 AND request=?2",
            params![scope.workspace_id, scope.request_id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    Ok(Some(read::passage(db, scope, repo, &id, max)?))
}
impl PassageStore for SqliteApplicationStore {
    fn load_text_preparation(
        &self,
        scope: &Scope,
        dataset_id: &str,
        extractor: &ExtractorIdentity,
        limits: &TextLimits,
        max_result_bytes: usize,
    ) -> Result<TextPreparation> {
        let limits = limits.effective(&self.limits)?;
        let key = json(extractor, HEADER_MAX)?;
        let repo = self.evidence.repository_identity()?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let dataset = read::dataset(&tx, scope, &repo, dataset_id, self.limits.max_output_bytes)?;
        if let Some(h) = cached(
            &tx,
            scope,
            &repo,
            dataset_id,
            &key,
            max_result_bytes.min(self.limits.max_output_bytes),
        )? {
            tx.commit().map_err(storage)?;
            return Ok(TextPreparation::Cached(Box::new(h)));
        }
        tx.commit().map_err(storage)?;
        let observation = dataset.document.as_ref().ok_or_else(corrupt)?;
        let bytes = self
            .evidence
            .document(observation, limits.max_input_bytes)?;
        if bytes.len() > limits.max_input_bytes {
            return Err(limit());
        }
        Ok(TextPreparation::Input(Box::new(TextPreparationInput {
            bytes,
            dataset,
            extractor: extractor.clone(),
            limits,
        })))
    }
    fn save_text_representation(
        &mut self,
        scope: &Scope,
        prepared: &PreparedText,
        max_result_bytes: usize,
    ) -> Result<TextRepresentation> {
        scope.validate()?;
        let max = max_result_bytes.min(self.limits.max_output_bytes);
        let repo = self.evidence.repository_identity()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        let dataset = read::dataset(
            &tx,
            scope,
            &repo,
            &prepared.dataset.id,
            self.limits.max_output_bytes,
        )?;
        if serde_json::to_value(&dataset).map_err(storage)?
            != serde_json::to_value(&prepared.dataset).map_err(storage)?
        {
            return Err(error(
                ErrorKind::StaleReference,
                "document dataset changed during preparation",
            ));
        }
        if let Some(existing) = cached(&tx, scope, &repo, &dataset.id, &prepared.extractor, max)? {
            tx.commit().map_err(storage)?;
            return Ok(existing);
        }
        let mut h = prepared.header.clone();
        h.id = self.ids.next_id();
        super::super::sqlite::validate_id(&h.id)?;
        h.created_at = self.clock.now();
        let payload = json(&h, max.min(HEADER_MAX))?;
        tx.execute("INSERT INTO app_records(id,workspace,repository,category,payload) VALUES(?1,?2,?3,'text',?4)",params![h.id,scope.workspace_id,repo,payload]).map_err(storage)?;
        tx.execute("INSERT INTO text_representations(id,workspace,repository,dataset,extractor,checksum) VALUES(?1,?2,?3,?4,?5,?6)",params![h.id,scope.workspace_id,repo,dataset.id,prepared.extractor,text_checksum(&payload)]).map_err(storage)?;
        {
            let mut stmt=tx.prepare("INSERT INTO text_chunks(representation,node,start,end,text,checksum) VALUES(?1,?2,?3,?4,?5,?6)").map_err(storage)?;
            for chunk in &prepared.chunks {
                stmt.execute(params![
                    h.id,
                    chunk.node,
                    chunk.start as i64,
                    chunk.end as i64,
                    chunk.text,
                    chunk.checksum
                ])
                .map_err(storage)?;
            }
        }
        {
            let mut stmt=tx.prepare("INSERT INTO text_nodes(representation,node,bytes,path,checksum) VALUES(?1,?2,?3,?4,?5)").map_err(storage)?;
            for node in &prepared.nodes {
                stmt.execute(params![
                    h.id,
                    i64::from(node.id),
                    node.bytes as i64,
                    node.path,
                    node.checksum
                ])
                .map_err(storage)?;
            }
        }
        {
            let mut stmt=tx.prepare("INSERT INTO text_mappings(representation,ordinal,start,end,payload,checksum) VALUES(?1,?2,?3,?4,?5,?6)").map_err(storage)?;
            for (ordinal, m) in prepared.mappings.iter().enumerate() {
                stmt.execute(params![
                    h.id,
                    ordinal as i64,
                    m.start as i64,
                    m.end as i64,
                    m.payload,
                    m.checksum
                ])
                .map_err(storage)?;
            }
        }
        tx.commit().map_err(storage)?;
        Ok(h)
    }
    fn read_text_representation(
        &self,
        scope: &Scope,
        id: &str,
        max_result_bytes: usize,
    ) -> Result<TextRepresentation> {
        let repo = self.evidence.repository_identity()?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let h = read::header(
            &tx,
            scope,
            &repo,
            id,
            max_result_bytes.min(self.limits.max_output_bytes),
        )?;
        tx.commit().map_err(storage)?;
        Ok(h)
    }
    fn read_text_page(
        &self,
        scope: &Scope,
        id: &str,
        start: usize,
        end: usize,
        limits: &TextLimits,
        max_result_bytes: usize,
    ) -> Result<TextPage> {
        let limits = limits.effective(&self.limits)?;
        let repo = self.evidence.repository_identity()?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let h = read::header(
            &tx,
            scope,
            &repo,
            id,
            HEADER_MAX.min(self.limits.max_output_bytes),
        )?;
        let text = read::chunks(&tx, id, -1, start, end, h.text_bytes, limits.max_page_bytes)?;
        let page = TextPage {
            representation_id: id.into(),
            start,
            end,
            total_bytes: h.text_bytes,
            text,
        };
        check_envelope(&page, max_result_bytes.min(self.limits.max_output_bytes))?;
        tx.commit().map_err(storage)?;
        Ok(page)
    }
    fn create_passage(
        &mut self,
        scope: &Scope,
        request: &CreatePassageRequest,
        limits: &TextLimits,
        max_result_bytes: usize,
    ) -> Result<Passage> {
        scope.validate()?;
        let limits = limits.effective(&self.limits)?;
        let max = max_result_bytes.min(self.limits.max_output_bytes);
        let input = json(request, self.limits.max_input_bytes)?;
        let repo = self.evidence.repository_identity()?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        if let Some(p) = duplicate_passage(&tx, scope, &repo, &input, max)? {
            tx.commit().map_err(storage)?;
            return Ok(p);
        }
        let id = self.id()?;
        let now = self.clock.now();
        if request.start >= request.end {
            return Err(invalid());
        }
        let h = read::header(
            &tx,
            scope,
            &repo,
            &request.representation_id,
            HEADER_MAX.min(self.limits.max_output_bytes),
        )?;
        let text = read::chunks(
            &tx,
            &h.id,
            -1,
            request.start,
            request.end,
            h.text_bytes,
            limits.max_passage_bytes.min(max),
        )?;
        let mappings = read::mappings(&tx, &h, request.start, request.end, max).map_err(|e| {
            if e.kind == ErrorKind::Storage {
                invalid()
            } else {
                e
            }
        })?;
        let p = build_passage(
            id,
            scope.clone(),
            h,
            request,
            &text,
            request.start,
            &mappings,
            now,
            &limits,
            max,
        )?;
        // Validate accessed source content and full result in one read snapshot.
        read::sources(&tx, &p, self.limits.max_output_bytes)?;
        let payload = json(&p, max)?;
        tx.commit().map_err(storage)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        if let Some(existing) = duplicate_passage(&tx, scope, &repo, &input, max)? {
            tx.commit().map_err(storage)?;
            return Ok(existing);
        }
        // Published representations are immutable. Recheck their trusted identity
        // after moving from the read snapshot to the short insertion transaction.
        let current = read::header(&tx, scope, &repo, &p.representation.id, HEADER_MAX)?;
        if serde_json::to_value(&current).map_err(storage)?
            != serde_json::to_value(&p.representation).map_err(storage)?
        {
            return Err(corrupt());
        }
        tx.execute("INSERT INTO app_records(id,workspace,repository,category,payload) VALUES(?1,?2,?3,'passage',?4)",params![p.id,scope.workspace_id,repo,payload]).map_err(storage)?;
        tx.execute("INSERT INTO passage_requests(workspace,request,input,passage,checksum) VALUES(?1,?2,?3,?4,?5)",params![scope.workspace_id,scope.request_id,input,p.id,text_checksum(&payload)]).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(p)
    }
    fn read_passage(&self, scope: &Scope, id: &str, max_result_bytes: usize) -> Result<Passage> {
        let repo = self.evidence.repository_identity()?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let p = read::passage(
            &tx,
            scope,
            &repo,
            id,
            max_result_bytes.min(self.limits.max_output_bytes),
        )?;
        tx.commit().map_err(storage)?;
        Ok(p)
    }
    fn resolve_passage_sources(
        &self,
        scope: &Scope,
        id: &str,
        max_result_bytes: usize,
    ) -> Result<PassageSource> {
        let max = max_result_bytes.min(self.limits.max_output_bytes);
        let repo = self.evidence.repository_identity()?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let passage = read::passage(&tx, scope, &repo, id, max)?;
        let remaining = max
            .checked_sub(check_envelope(&passage, max)?)
            .and_then(|n| n.checked_sub(23))
            .ok_or_else(limit)?;
        let sources = read::sources(&tx, &passage, remaining)?;
        let result = PassageSource { passage, sources };
        check_envelope(&result, max)?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
}
