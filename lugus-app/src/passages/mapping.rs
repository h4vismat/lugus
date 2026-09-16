use super::domain::*;
use crate::{Result, Scope};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub fn text_checksum(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn validate_selection<'a>(
    text: &'a str,
    start: usize,
    end: usize,
    expected: &str,
) -> Result<&'a str> {
    if start >= end || expected.is_empty() {
        return Err(invalid("selection must be nonempty"));
    }
    let selected = text
        .get(start..end)
        .ok_or_else(|| invalid("invalid UTF-8 text range"))?;
    if selected != expected {
        return Err(invalid("selected text does not match expected text"));
    }
    Ok(selected)
}

/// Validate a bounded loaded slice against its absolute canonical coordinates.
pub fn validate_selection_slice<'a>(
    text: &'a str,
    slice_start: usize,
    request: &CreatePassageRequest,
    limits: &TextLimits,
) -> Result<&'a str> {
    limits.validate()?;
    if request.expected_text.len() > limits.max_passage_bytes {
        return Err(limit());
    }
    let start = request
        .start
        .checked_sub(slice_start)
        .ok_or_else(|| invalid("invalid selection range"))?;
    let end = request
        .end
        .checked_sub(slice_start)
        .ok_or_else(|| invalid("invalid selection range"))?;
    validate_selection(text, start, end, &request.expected_text)
}

pub fn text_page(
    text: &str,
    representation_id: &str,
    start: usize,
    end: usize,
    limits: &TextLimits,
) -> Result<TextPage> {
    limits.validate()?;
    let length = end
        .checked_sub(start)
        .ok_or_else(|| invalid("invalid text range"))?;
    if length > limits.max_page_bytes {
        return Err(limit());
    }
    let text_slice = text
        .get(start..end)
        .ok_or_else(|| invalid("invalid UTF-8 text range"))?;
    Ok(TextPage {
        representation_id: representation_id.into(),
        start,
        end,
        total_bytes: text.len(),
        text: text_slice.into(),
    })
}

/// Accepts just the indexed intersecting mapping rows. Coordinates remain absolute.
/// Normalized rows may overlap: each retains one contributing source interval.
pub fn clip_mappings(
    mappings: &[SourceMapping],
    start: usize,
    end: usize,
) -> Result<Vec<SourceMapping>> {
    if start >= end {
        return Err(invalid("selection must be nonempty"));
    }
    let mut result = Vec::new();
    let mut covered = start;
    let mut backed = false;
    for mapping in mappings {
        if mapping.start >= mapping.end {
            return Err(invalid("invalid stored mapping"));
        }
        if mapping.end <= start || mapping.start >= end {
            continue;
        }
        let mut clipped = mapping.clone();
        clipped.start = start.max(mapping.start);
        clipped.end = end.min(mapping.end);
        if clipped.start > covered {
            return Err(invalid("incomplete source mappings"));
        }
        covered = covered.max(clipped.end);
        match (&mut clipped.source, mapping.kind) {
            (Some(source), MappingKind::Exact) => {
                if source.end.checked_sub(source.start) != mapping.end.checked_sub(mapping.start) {
                    return Err(invalid("invalid exact source mapping"));
                }
                source.start = source
                    .start
                    .checked_add(clipped.start - mapping.start)
                    .ok_or_else(limit)?;
                source.end = source
                    .start
                    .checked_add(clipped.end - clipped.start)
                    .ok_or_else(limit)?;
                backed = true;
            }
            (Some(source), MappingKind::Normalized) if source.start < source.end => backed = true,
            (None, MappingKind::Synthetic) => {}
            _ => return Err(invalid("invalid source mapping")),
        }
        result.push(clipped);
    }
    if covered != end || !backed {
        return Err(invalid("selection has no complete source backing"));
    }
    Ok(result)
}

/// Resolution for in-memory preparation/tests; stores should read only these intervals.
pub fn resolve_sources(
    nodes: &[SourceNode],
    mappings: &[SourceMapping],
    max_bytes: usize,
) -> Result<Vec<SourceExcerpt>> {
    let index: HashMap<_, _> = nodes.iter().map(|node| (node.node_id, node)).collect();
    let mut result = Vec::new();
    let mut serialized_bytes = 2usize;
    if serialized_bytes > max_bytes {
        return Err(limit());
    }
    for mapping in mappings {
        if let Some(source) = &mapping.source {
            let node = index
                .get(&source.node_id)
                .ok_or_else(|| invalid("source node unavailable"))?;
            let text = node
                .text
                .get(source.start..source.end)
                .ok_or_else(|| invalid("invalid source range"))?;
            if text.len() > max_bytes {
                return Err(limit());
            }
            let excerpt = SourceExcerpt {
                node_id: source.node_id,
                path: node.path.clone(),
                start: source.start,
                end: source.end,
                text: text.into(),
            };
            serialized_bytes = serialized_bytes
                .checked_add(check_envelope(&excerpt, max_bytes)?)
                .and_then(|n| n.checked_add(usize::from(!result.is_empty())))
                .filter(|n| *n <= max_bytes)
                .ok_or_else(limit)?;
            result.push(excerpt);
        }
    }
    Ok(result)
}

/// Build from trusted stored metadata, bounded canonical slice and indexed mapping rows.
#[allow(clippy::too_many_arguments)]
pub fn build_passage(
    id: String,
    scope: Scope,
    representation: TextRepresentation,
    request: &CreatePassageRequest,
    text_slice: &str,
    slice_start: usize,
    mappings: &[SourceMapping],
    created_at: DateTime<Utc>,
    limits: &TextLimits,
    max_result_bytes: usize,
) -> Result<Passage> {
    scope.validate()?;
    if representation.workspace_id != scope.workspace_id {
        return Err(crate::AppError::new(
            crate::ErrorKind::ScopeMismatch,
            "passage workspace mismatch",
            false,
        ));
    }
    if representation.id != request.representation_id {
        return Err(invalid("passage representation mismatch"));
    }
    if request.end > representation.text_bytes {
        return Err(invalid("invalid selection range"));
    }
    let quote = validate_selection_slice(text_slice, slice_start, request, limits)?.to_owned();
    let passage = Passage {
        id,
        scope,
        representation,
        start: request.start,
        end: request.end,
        quote_checksum: text_checksum(&quote),
        quote,
        mappings: clip_mappings(mappings, request.start, request.end)?,
        created_at,
    };
    check_envelope(&passage, max_result_bytes)?;
    Ok(passage)
}

/// Counts actual JSON escaping overhead without allocating the serialized envelope.
pub fn check_envelope(value: &impl serde::Serialize, max_bytes: usize) -> Result<usize> {
    struct Counter {
        bytes: usize,
        max: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes = self
                .bytes
                .checked_add(bytes.len())
                .filter(|n| *n <= self.max)
                .ok_or_else(|| std::io::Error::other("bounded JSON limit"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter {
        bytes: 0,
        max: max_bytes,
    };
    serde_json::to_writer(&mut counter, value).map_err(|_| limit())?;
    Ok(counter.bytes)
}

/// Validate a replaceable adapter's result before accepting it as trusted stored text.
pub fn validate_extracted(
    extracted: &ExtractedText,
    limits: &TextLimits,
    cancellation: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    limits.validate()?;
    check_cancel(cancellation)?;
    if extracted.text.len() > limits.max_text_bytes
        || extracted.source_nodes.len() > limits.max_nodes
        || extracted.mappings.len() > limits.max_mappings
    {
        return Err(limit());
    }
    check_envelope(
        &(&extracted.source_nodes, &extracted.mappings),
        limits.max_source_bytes,
    )?;
    check_envelope(
        &(
            &extracted.extractor,
            &extracted.decoder,
            &extracted.limitations,
        ),
        if limits.max_text_bytes == isize::MAX as usize {
            limits.max_source_bytes
        } else {
            limits.max_source_bytes.min(64 * 1024)
        },
    )?;
    if extracted.text_checksum != text_checksum(&extracted.text) {
        return Err(invalid("canonical text checksum mismatch"));
    }
    let mut index = HashMap::new();
    let mut paths = std::collections::HashSet::new();
    for node in &extracted.source_nodes {
        check_cancel(cancellation)?;
        if node.path.is_empty()
            || node.path.len() > limits.max_depth
            || index.insert(node.node_id, node).is_some()
            || !paths.insert(&node.path)
        {
            return Err(invalid("invalid source node identity"));
        }
    }
    let mut covered = 0;
    let mut previous: Option<&SourceMapping> = None;
    let mut backed = false;
    for mapping in &extracted.mappings {
        check_cancel(cancellation)?;
        if mapping.start >= mapping.end || mapping.start > covered {
            return Err(invalid("incomplete source mappings"));
        }
        if mapping.start < covered
            && !previous.is_some_and(|p| {
                p.start == mapping.start
                    && p.end == mapping.end
                    && p.kind == MappingKind::Normalized
                    && mapping.kind == MappingKind::Normalized
            })
        {
            return Err(invalid("invalid overlapping source mappings"));
        }
        let canonical = extracted
            .text
            .get(mapping.start..mapping.end)
            .ok_or_else(|| invalid("invalid canonical source range"))?;
        match (&mapping.source, mapping.kind) {
            (Some(source), kind @ (MappingKind::Exact | MappingKind::Normalized)) => {
                if source.start >= source.end {
                    return Err(invalid("invalid source range"));
                }
                let node = index
                    .get(&source.node_id)
                    .ok_or_else(|| invalid("source node unavailable"))?;
                let original = node
                    .text
                    .get(source.start..source.end)
                    .ok_or_else(|| invalid("invalid source range"))?;
                if (kind == MappingKind::Exact && original != canonical)
                    || (kind == MappingKind::Normalized
                        && (canonical != " " || !original.chars().all(char::is_whitespace)))
                {
                    return Err(invalid("source text does not match mapping"));
                }
                backed = true;
            }
            (None, MappingKind::Synthetic) if canonical == "\n" || canonical == "\t" => {}
            _ => return Err(invalid("invalid source mapping")),
        }
        covered = mapping.end;
        previous = Some(mapping);
    }
    if covered != extracted.text.len() || (!extracted.text.is_empty() && !backed) {
        return Err(invalid("canonical text has no complete source backing"));
    }
    check_cancel(cancellation)
}

/// Derive immutable metadata from a scoped, trusted document dataset and validated extraction.
/// The caller invokes `validate_extracted` and checks its own metadata/result envelope.
pub fn build_representation(
    id: String,
    dataset: &crate::DatasetHeader,
    extracted: &ExtractedText,
    created_at: DateTime<Utc>,
) -> Result<TextRepresentation> {
    if id.trim().is_empty() || id.len() > Scope::MAX_ID_BYTES || id.chars().any(char::is_control) {
        return Err(invalid("invalid representation identity"));
    }
    if dataset.kind != crate::DatasetKind::Document
        || !matches!(dataset.projection, crate::DatasetProjection::Document)
        || dataset.error.is_some()
    {
        return Err(invalid("text requires a successful document dataset"));
    }
    let document = dataset
        .document
        .clone()
        .ok_or_else(|| invalid("document observation unavailable"))?;
    if document.provider != dataset.provider {
        return Err(invalid("document provider mismatch"));
    }
    Ok(TextRepresentation {
        id,
        workspace_id: dataset.workspace_id.clone(),
        repository_id: dataset.repository_id.clone(),
        dataset_id: dataset.id.clone(),
        document,
        extractor: extracted.extractor.clone(),
        decoder: extracted.decoder.clone(),
        text_checksum: extracted.text_checksum.clone(),
        text_bytes: extracted.text.len(),
        source_node_count: extracted.source_nodes.len(),
        mapping_count: extracted.mappings.len(),
        limitations: extracted.limitations.clone(),
        created_at,
    })
}
