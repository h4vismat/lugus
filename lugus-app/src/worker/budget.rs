//! A single operation budget survives resolution fallbacks and all ingestion pages.
use crate::{AppError, ErrorKind, Limits, ManagedProvider, PriceQuery, ProviderIdentity, Query};
use async_trait::async_trait;
use lugus_financial::{
    capabilities::*,
    domain::{Document, Fact, Filing, Page},
    error::{Error as FinancialError, ErrorKind as FinancialKind, Result},
    market_data::PricePage,
    resolution::{Candidate, LookupRequest, ResolutionPage, SearchRequest},
};
use std::{future::Future, io::Write};
use tokio::{sync::watch, time::Instant};

pub(super) async fn cancelled(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
pub(super) struct Budget {
    pub limits: Limits,
    pub deadline: Instant,
    pub cancel: watch::Receiver<bool>,
    pub shutdown: watch::Receiver<bool>,
    pub cause: Option<AppError>,
    pages: usize,
    items: usize,
    bytes: usize,
}
impl Budget {
    pub fn new(
        limits: Limits,
        deadline: Instant,
        cancel: watch::Receiver<bool>,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self {
            limits,
            deadline,
            cancel,
            shutdown,
            cause: None,
            pages: 0,
            items: 0,
            bytes: 0,
        }
    }
    fn fail(&mut self, kind: ErrorKind) -> FinancialError {
        let (message, financial) = match kind {
            ErrorKind::Cancelled => ("provider operation cancelled", FinancialKind::Unavailable),
            ErrorKind::Timeout => (
                "provider operation deadline exceeded",
                FinancialKind::Timeout,
            ),
            _ => (
                "provider fetch budget exceeded",
                FinancialKind::InvalidRequest,
            ),
        };
        self.cause = Some(AppError::new(kind, message, false));
        FinancialError::new(financial, message)
    }
    pub async fn call<T>(&mut self, future: impl Future<Output = Result<T>>) -> Result<T> {
        if self.pages >= self.limits.max_pages_per_fetch {
            return Err(self.fail(ErrorKind::ResourceLimit));
        }
        self.pages += 1;
        tokio::select! {
            biased;
            _ = cancelled(&mut self.shutdown) => Err(self.fail(ErrorKind::Cancelled)),
            _ = cancelled(&mut self.cancel) => Err(self.fail(ErrorKind::Cancelled)),
            _ = tokio::time::sleep_until(self.deadline) => Err(self.fail(ErrorKind::Timeout)),
            result = future => {
                if result.as_ref().is_err_and(|e| e.kind == FinancialKind::Protocol && matches!(e.message.as_str(), "response exceeds size limit" | "document exceeds size limit")) {
                    self.cause = Some(AppError::new(ErrorKind::ResourceLimit, "provider response exceeds byte budget", false));
                }
                result
            },
        }
    }
    fn accept(&mut self, value: &impl serde::Serialize, items: usize) -> Result<()> {
        let mut size = Counter {
            used: 0,
            max: self.limits.max_bytes_per_fetch.saturating_sub(self.bytes),
        };
        if self.items.saturating_add(items) > self.limits.max_items_per_fetch
            || serde_json::to_writer(&mut size, value).is_err()
        {
            return Err(self.fail(ErrorKind::ResourceLimit));
        }
        self.bytes += size.used;
        self.items += items;
        Ok(())
    }
}
struct Counter {
    used: usize,
    max: usize,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.max.saturating_sub(self.used) {
            return Err(std::io::Error::other("byte limit"));
        }
        self.used += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) struct BoundedProvider<'a> {
    pub inner: &'a mut dyn ManagedProvider,
    pub budget: Budget,
}
impl Provider for BoundedProvider<'_> {
    fn identity(&self) -> &ProviderIdentity {
        self.inner.identity()
    }
}
#[async_trait]
impl FilingsProvider for BoundedProvider<'_> {
    async fn list_filings(&mut self, query: &Query) -> Result<Page<Filing>> {
        let page = self.budget.call(self.inner.list_filings(query)).await?;
        self.budget.accept(&page, page.items.len())?;
        Ok(page)
    }
    async fn fetch_document(&mut self, source_url: &str, max_bytes: usize) -> Result<Document> {
        let document = self
            .budget
            .call(self.inner.fetch_document(source_url, max_bytes))
            .await?;
        if document
            .bytes(self.budget.limits.max_document_bytes)
            .is_err()
        {
            return Err(self.budget.fail(ErrorKind::ResourceLimit));
        }
        self.budget.accept(&document, 1)?;
        Ok(document)
    }
}
#[async_trait]
impl FundamentalsProvider for BoundedProvider<'_> {
    async fn fetch_facts(&mut self, query: &Query) -> Result<Page<Fact>> {
        let page = self.budget.call(self.inner.fetch_facts(query)).await?;
        self.budget.accept(&page, page.items.len())?;
        Ok(page)
    }
}
#[async_trait]
impl MarketDataProvider for BoundedProvider<'_> {
    async fn fetch_prices(&mut self, query: &PriceQuery) -> Result<PricePage> {
        let page = self.budget.call(self.inner.fetch_prices(query)).await?;
        self.budget.accept(&page, page.items.len())?;
        Ok(page)
    }
}
#[async_trait]
impl CompanyResolutionProvider for BoundedProvider<'_> {
    async fn search_companies(&mut self, request: &SearchRequest) -> Result<ResolutionPage> {
        let page = self
            .budget
            .call(self.inner.search_companies(request))
            .await?;
        self.budget.accept(&page, page.items.len())?;
        Ok(page)
    }
    async fn lookup_company(&mut self, request: &LookupRequest) -> Result<Candidate> {
        let candidate = self.budget.call(self.inner.lookup_company(request)).await?;
        self.budget.accept(&candidate, 1)?;
        Ok(candidate)
    }
}
