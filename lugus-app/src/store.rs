//! Application persistence port and bounded SQLite adapter.
mod binding_history;
mod binding_reads;
mod bindings;
mod conversations;
mod evidence;
mod freeze;
mod history_evidence;
mod passages;
pub use passages::{PassageStore, PreparedText, TextPreparation, TextPreparationInput};
mod comparison;
mod portfolio;
mod sqlite;
mod views;
use crate::*;
pub use evidence::EvidenceRepository;
use lugus_financial::resolution::catalog::CatalogSelection;
pub use sqlite::SqliteApplicationStore;

pub trait ApplicationStore: Send {
    fn comparison_store(&self) -> Result<&dyn crate::comparison::ComparisonStore> {
        Err(error(
            ErrorKind::Unsupported,
            "comparison storage unavailable",
        ))
    }
    fn comparison_store_mut(&mut self) -> Result<&mut dyn crate::comparison::ComparisonStore> {
        Err(error(
            ErrorKind::Unsupported,
            "comparison storage unavailable",
        ))
    }

    fn history_evidence_page(
        &self,
        _scope: &Scope,
        _id: &str,
        _page: PageRequest,
    ) -> Result<lugus_financial::storage::history::HistoryReadPage> {
        Err(error(
            ErrorKind::Unsupported,
            "historical evidence unavailable",
        ))
    }

    fn portfolio_store(&self) -> Result<&dyn crate::portfolio::PortfolioStore> {
        Err(error(
            ErrorKind::Unsupported,
            "portfolio storage unavailable",
        ))
    }
    fn portfolio_store_mut(&mut self) -> Result<&mut dyn crate::portfolio::PortfolioStore> {
        Err(error(
            ErrorKind::Unsupported,
            "portfolio storage unavailable",
        ))
    }

    fn passage_store(&self) -> Result<&dyn PassageStore> {
        Err(error(
            ErrorKind::Unsupported,
            "passage storage is unavailable",
        ))
    }
    fn passage_store_mut(&mut self) -> Result<&mut dyn PassageStore> {
        Err(error(
            ErrorKind::Unsupported,
            "passage storage is unavailable",
        ))
    }
    fn conversation_store(&self) -> Result<&dyn crate::conversations::ConversationStore> {
        Err(error(
            ErrorKind::Unsupported,
            "conversation storage is unavailable",
        ))
    }
    fn conversation_store_mut(
        &mut self,
    ) -> Result<&mut dyn crate::conversations::ConversationStore> {
        Err(error(
            ErrorKind::Unsupported,
            "conversation storage is unavailable",
        ))
    }
    fn bind(&mut self, scope: &Scope, request: &BindRequest) -> Result<BindingRecord>;
    fn read_binding(&self, scope: &Scope, id: &str) -> Result<BindingView>;
    fn list_bindings(&self, scope: &Scope, page: PageRequest) -> Result<BindingPage>;
    fn revoke_binding(
        &mut self,
        scope: &Scope,
        request: &RevokeBindingRequest,
    ) -> Result<BindingView>;
    fn prepare_binding(&self, scope: &Scope, id: &str) -> Result<BindingRecord>;
    fn binding_history(
        &self,
        scope: &Scope,
        id: &str,
        page: PageRequest,
    ) -> Result<BindingHistoryPage>;

    /// Trusted host import: never expose raw run receipt registration as an agent tool.
    fn record_fetch(&mut self, result: &FetchResult) -> Result<FetchReference>;
    fn read_fetch(&self, scope: &Scope, id: &str) -> Result<FetchReference>;
    fn create_dataset(
        &mut self,
        scope: &Scope,
        fetch_id: &str,
        projection: DatasetProjection,
    ) -> Result<DatasetHeader>;
    fn dataset_header(&self, scope: &Scope, id: &str) -> Result<DatasetHeader>;
    fn read_dataset(&self, scope: &Scope, id: &str, page: PageRequest) -> Result<DatasetPage>;
    fn read_document(
        &self,
        scope: &Scope,
        id: &str,
        offset: usize,
        length: usize,
    ) -> Result<DocumentRead>;
    fn select_candidate(
        &mut self,
        scope: &Scope,
        id: &str,
        observation_id: i64,
    ) -> Result<CatalogSelection>;
    fn open_view(&mut self, scope: &Scope, request: &OpenViewRequest) -> Result<ViewReceipt>;
    fn read_view(&self, scope: &Scope, id: &str) -> Result<ViewReceipt>;
    fn report_presentation(&mut self, scope: &Scope, result: &PresentationResult) -> Result<()>;
}
fn error(kind: ErrorKind, message: &'static str) -> AppError {
    AppError::new(kind, message, false)
}
fn storage(_: impl std::fmt::Display) -> AppError {
    error(ErrorKind::Storage, "application storage operation failed")
}
fn limit() -> AppError {
    error(
        ErrorKind::ResourceLimit,
        "reference exceeds configured bounds",
    )
}
fn json<T: serde::Serialize>(value: &T, max: usize) -> Result<String> {
    // Write through a capped sink so serialization cannot allocate unbounded output.
    struct Capped {
        bytes: Vec<u8>,
        max: usize,
    }
    impl std::io::Write for Capped {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other("limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Capped {
        bytes: Vec::new(),
        max,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| limit())?;
    String::from_utf8(writer.bytes).map_err(storage)
}
fn safe_error(error: &Option<AppError>) -> Option<AppError> {
    error.as_ref().map(|e| AppError {
        kind: e.kind,
        message: "fetch did not complete successfully".into(),
        retryable: e.retryable,
        retry_after_seconds: e.retry_after_seconds,
    })
}
