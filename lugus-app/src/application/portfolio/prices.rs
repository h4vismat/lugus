use super::*;
use lugus_financial::{market_data::PriceQuery, selection::PriceSeries};
pub(in crate::application) struct PortfolioJob {
    pub(in crate::application) cancel: tokio::sync::watch::Sender<bool>,
    pub(in crate::application) done: tokio::sync::watch::Receiver<bool>,
}
impl Application {
    /// Admission is durable; execution is bounded and owned through application shutdown.
    pub async fn refresh_portfolio(&self, request: RefreshRequest) -> Result<RefreshResult> {
        let r = request.clone();
        let (mut result, started) = self
            .portfolio_effect(move |s| s.portfolio_refresh_begin(&r))
            .await?;
        if !started {
            return self.portfolio_refresh_status(result.id).await;
        }
        let (cancel, mut cancelled) = watch::channel(false);
        let (done_sender, done) = watch::channel(false);
        let admitted = {
            let mut admission = lock(&self.inner.admission)?;
            if admission.closed || admission.refreshes.len() >= 4 {
                false
            } else {
                admission
                    .refreshes
                    .insert(result.id.clone(), PortfolioJob { cancel, done });
                true
            }
        };
        if !admitted {
            result.status = "cancelled".into();
            return self
                .portfolio_effect(move |s| s.portfolio_refresh_finish(&result))
                .await;
        }
        let receipt = result.clone();
        let app = self.clone();
        tokio::spawn(async move {
            let outcome = tokio::select! {
                biased;
                _=async {while !*cancelled.borrow_and_update(){if cancelled.changed().await.is_err(){break;}}} => Some("cancelled"),
                _=tokio::time::sleep(std::time::Duration::from_secs(60)) => Some("timed_out"),
                collected=app.collect_portfolio_prices(&mut result) => if collected.is_err(){Some("failed")}else{None},
            };
            if let Some(status) = outcome {
                result.status = status.into();
            }
            let id = result.id.clone();
            let _ = app
                .portfolio_effect(move |s| s.portfolio_refresh_finish(&result))
                .await;
            done_sender.send_replace(true);
            if let Ok(mut admission) = lock(&app.inner.admission) {
                admission.refreshes.remove(&id);
            }
        });
        Ok(receipt)
    }
    pub fn cancel_portfolio_refresh(&self, id: String) -> Result<()> {
        let admission = lock(&self.inner.admission)?;
        if let Some(job) = admission.refreshes.get(&id) {
            job.cancel.send_replace(true);
        }
        Ok(())
    }
    async fn collect_portfolio_prices(&self, result: &mut RefreshResult) -> Result<()> {
        let doc = self
            .portfolio_document(result.request.portfolio_id.clone())
            .await?;
        let today = chrono::Utc::now().date_naive();
        for instrument in &doc.instruments {
            let Some(binding) = &instrument.binding else {
                continue;
            };
            let mut receipt = PortfolioPriceReceipt {
                instrument_id: instrument.id.clone(),
                binding: binding.clone(),
                fetch_id: None,
                provider: None,
                observed_at: chrono::Utc::now(),
                price: None,
                raw_bar: None,
                status: String::new(),
            };
            let fetched = async {
                let scope = self.scope(
                    &format!("portfolio:{}", doc.header.id),
                    &format!("{}:{}", result.id, instrument.id),
                    None,
                )?;
                let query = PriceQuery {
                    instrument: binding.native_id.clone(),
                    start: today - chrono::Duration::days(14),
                    end: today,
                    cursor: None,
                    page_size: 100,
                };
                let job = self.submit_manual(
                    &scope,
                    FetchCommand::Prices {
                        instance_id: binding.instance_id.clone(),
                        query: query.clone(),
                    },
                )?;
                let mut guard = RefreshJobGuard {
                    application: self.clone(),
                    scope: scope.clone(),
                    id: Some(job.id.clone()),
                };
                let status = self.wait(&scope, &job.id).await?;
                guard.id = None;
                let fetch_id = status.fetch_id.ok_or_else(|| {
                    crate::portfolio::invalid("provider did not save a fetch receipt")
                })?;
                receipt.fetch_id = Some(fetch_id.clone());
                let fetch = self.read_fetch(&scope, &fetch_id).await?;
                receipt.provider = Some(fetch.provider.clone());
                if let Some(error) = fetch.error {
                    return Err(error);
                }
                let run_id = fetch
                    .runs
                    .iter()
                    .find(|r| r.kind == RunKind::Market)
                    .ok_or_else(|| crate::portfolio::invalid("no market run saved"))?
                    .id;
                let dataset = self
                    .create_dataset(
                        &scope,
                        &fetch_id,
                        DatasetProjection::Prices {
                            run_id,
                            query,
                            series: PriceSeries::Close,
                        },
                    )
                    .await?;
                if !dataset.conflicts.is_empty(){return Err(crate::portfolio::invalid("conflicting price observations"));}
                let mut offset = 0;
                let mut bars = vec![];
                loop {
                    let page = self
                        .read_dataset(&scope, &dataset.id, PageRequest { offset, limit: 100 })
                        .await?;
                    for row in page.rows {
                        if let DatasetRow::Price { evidence, .. } = row {
                            bars.push(evidence);
                        }
                    }
                    match page.next_offset {
                        Some(next) if next > offset && next <= 1000 => offset = next,
                        Some(_) => {
                            return Err(crate::portfolio::invalid(
                                "price evidence exceeds refresh limit",
                            ));
                        }
                        None => break,
                    }
                }
                bars.sort_by_key(|b| b.value.date);
                let observation = bars
                    .pop()
                    .ok_or_else(|| crate::portfolio::invalid("no daily price observations"))?;
                let bar=observation.value;
                receipt.raw_bar = Some(bar.clone());
                let last_split=doc.accounts.iter().flat_map(|a|a.ledger.events.iter()).filter(|e|matches!(&e.kind,lugus_portfolio::EventKind::Split{instrument_id:id,..} if id==&instrument.id)).map(|e|e.date).max();
                receipt.price=crate::portfolio::yfinance_price_input(&instrument.id,&fetch.provider,binding,&bar,observation.retrieval.observation_id,last_split,today);
                receipt.status = if receipt.price.is_some() {
                    format!("Close {} on {}; yfinance split-only close, retrieved {}; no later recorded split",bar.close.as_str(),bar.date,bar.retrieved_at)
                } else if bar.currency != "USD" {
                    "Price is not denominated in USD".into()
                } else {
                    format!(
                        "Close {} on {}; share-price basis unverified",
                        bar.close.as_str(),
                        bar.date
                    )
                };
                Ok(())
            }
            .await;
            if let Err(error) = fetched {
                receipt.status = error.message;
            }
            result.receipts.push(receipt);
        }
        result.status = if doc.instruments.iter().all(|i| i.binding.is_some())
            && result.receipts.iter().all(|r| r.price.is_some())
        {
            "complete"
        } else {
            "partial"
        }
        .into();
        Ok(())
    }
    pub async fn portfolio_refresh_status(&self, id: String) -> Result<RefreshResult> {
        let saved_id = id.clone();
        let mut result = self
            .portfolio_effect(move |s| s.portfolio_refresh_read(&saved_id))
            .await?;
        if result.status == "running" && !lock(&self.inner.admission)?.refreshes.contains_key(&id) {
            // A different process may own the job, or the previous process died.
            // Do not claim it is live here or overwrite another process's receipt.
            result.status = "interrupted_or_external".into();
        }
        Ok(result)
    }
}
struct RefreshJobGuard {
    application: Application,
    scope: Scope,
    id: Option<String>,
}
impl Drop for RefreshJobGuard {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = self.application.cancel(&self.scope, &id);
        }
    }
}
