//! Scoped storage port. Preparation tokens are built outside the store lock.
use super::*;
use crate::passages::*;
use rusqlite::{Connection, OptionalExtension, params};
mod prepared;
mod read;
mod write;
pub use prepared::{PreparedText, TextPreparation, TextPreparationInput};

pub trait PassageStore: Send {
    fn load_text_preparation(
        &self,
        scope: &Scope,
        dataset_id: &str,
        extractor: &ExtractorIdentity,
        limits: &TextLimits,
        max_result_bytes: usize,
    ) -> Result<TextPreparation>;
    fn save_text_representation(
        &mut self,
        scope: &Scope,
        prepared: &PreparedText,
        max_result_bytes: usize,
    ) -> Result<TextRepresentation>;
    fn read_text_representation(
        &self,
        scope: &Scope,
        id: &str,
        max_result_bytes: usize,
    ) -> Result<TextRepresentation>;
    fn read_text_page(
        &self,
        scope: &Scope,
        id: &str,
        start: usize,
        end: usize,
        limits: &TextLimits,
        max_result_bytes: usize,
    ) -> Result<TextPage>;
    fn create_passage(
        &mut self,
        scope: &Scope,
        request: &CreatePassageRequest,
        limits: &TextLimits,
        max_result_bytes: usize,
    ) -> Result<Passage>;
    fn read_passage(&self, scope: &Scope, id: &str, max_result_bytes: usize) -> Result<Passage>;
    fn resolve_passage_sources(
        &self,
        scope: &Scope,
        id: &str,
        max_result_bytes: usize,
    ) -> Result<PassageSource>;
}
const CHUNK: usize = 4096;
const HEADER_MAX: usize = 64 * 1024;
fn corrupt() -> AppError {
    error(ErrorKind::Storage, "stored text integrity check failed")
}
fn invalid() -> AppError {
    error(ErrorKind::InvalidInput, "invalid text selection")
}
fn bounded(value: i64, maximum: usize) -> Result<usize> {
    usize::try_from(value)
        .ok()
        .filter(|v| *v <= maximum)
        .ok_or_else(limit)
}
pub(super) fn migrate(tx: &Connection) -> Result<()> {
    tx.execute_batch("CREATE TABLE text_representations(id TEXT PRIMARY KEY REFERENCES app_records(id),workspace TEXT NOT NULL,repository TEXT NOT NULL,dataset TEXT NOT NULL REFERENCES app_records(id),extractor TEXT NOT NULL,checksum TEXT NOT NULL,UNIQUE(workspace,repository,dataset,extractor));
CREATE TABLE text_chunks(representation TEXT NOT NULL REFERENCES text_representations(id),node INTEGER NOT NULL,start INTEGER NOT NULL,end INTEGER NOT NULL,text TEXT NOT NULL,checksum TEXT NOT NULL,PRIMARY KEY(representation,node,start));
CREATE INDEX text_chunk_ends ON text_chunks(representation,node,end);
CREATE TABLE text_nodes(representation TEXT NOT NULL REFERENCES text_representations(id),node INTEGER NOT NULL,bytes INTEGER NOT NULL,path TEXT NOT NULL,checksum TEXT NOT NULL,PRIMARY KEY(representation,node));
CREATE TABLE text_mappings(representation TEXT NOT NULL REFERENCES text_representations(id),ordinal INTEGER NOT NULL,start INTEGER NOT NULL,end INTEGER NOT NULL,payload TEXT NOT NULL,checksum TEXT NOT NULL,PRIMARY KEY(representation,ordinal));
CREATE INDEX text_mapping_starts ON text_mappings(representation,start);
CREATE INDEX text_mapping_ends ON text_mappings(representation,end);
CREATE TABLE passage_requests(workspace TEXT NOT NULL,request TEXT NOT NULL,input TEXT NOT NULL,passage TEXT NOT NULL UNIQUE REFERENCES app_records(id),checksum TEXT NOT NULL,PRIMARY KEY(workspace,request));
PRAGMA user_version=4;").map_err(storage)
}
