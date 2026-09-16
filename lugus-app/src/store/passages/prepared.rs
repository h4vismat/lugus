use super::*;
use std::sync::atomic::AtomicBool;
#[derive(Debug)]
pub enum TextPreparation {
    Cached(Box<TextRepresentation>),
    Input(Box<TextPreparationInput>),
}
#[derive(Debug)]
pub struct TextPreparationInput {
    pub bytes: Vec<u8>,
    pub(super) dataset: DatasetHeader,
    pub(super) extractor: ExtractorIdentity,
    pub(super) limits: TextLimits,
}
impl TextPreparationInput {
    pub fn dataset(&self) -> &DatasetHeader {
        &self.dataset
    }
    pub fn media_type(&self) -> &str {
        &self
            .dataset
            .document
            .as_ref()
            .expect("authorized document")
            .media_type
    }
}
pub(super) struct Chunk {
    pub node: i64,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub checksum: String,
}
pub(super) struct Node {
    pub id: u32,
    pub bytes: usize,
    pub path: String,
    pub checksum: String,
}
pub(super) struct Mapping {
    pub start: usize,
    pub end: usize,
    pub payload: String,
    pub checksum: String,
}
/// Private fields prevent unvalidated adapter output from reaching persistence.
/// Build on the host's bounded blocking worker, after extraction and outside its store mutex.
pub struct PreparedText {
    pub(super) dataset: DatasetHeader,
    pub(super) extractor: String,
    pub(super) header: TextRepresentation,
    pub(super) chunks: Vec<Chunk>,
    pub(super) nodes: Vec<Node>,
    pub(super) mappings: Vec<Mapping>,
}
impl PreparedText {
    pub fn new(
        input: TextPreparationInput,
        extracted: ExtractedText,
        limits: &TextLimits,
        cancellation: &AtomicBool,
    ) -> Result<Self> {
        limits.validate()?;
        if *limits != input.limits {
            return Err(error(ErrorKind::InvalidInput, "preparation limits changed"));
        }
        if extracted.extractor != input.extractor {
            return Err(error(
                ErrorKind::InvalidInput,
                "extractor identity mismatch",
            ));
        }
        validate_extracted(&extracted, limits, cancellation)?;
        let header = build_representation(
            "pending".into(),
            &input.dataset,
            &extracted,
            input.dataset.created_at,
        )?;
        check_envelope(
            &header,
            if limits.max_text_bytes == isize::MAX as usize {
                isize::MAX as usize
            } else {
                HEADER_MAX
            },
        )?;
        let mut chunks = Vec::new();
        let mut add = |node: i64, text: &str| -> Result<()> {
            let mut start = 0;
            while start < text.len() {
                if cancellation.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(error(ErrorKind::Cancelled, "text preparation cancelled"));
                }
                let mut end = (start + CHUNK).min(text.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                let text = text[start..end].to_owned();
                let checksum = text_checksum(&text);
                chunks.push(Chunk {
                    node,
                    start,
                    end,
                    text,
                    checksum,
                });
                start = end;
            }
            Ok(())
        };
        add(-1, &extracted.text)?;
        let mut nodes = Vec::with_capacity(extracted.source_nodes.len());
        for node in &extracted.source_nodes {
            add(i64::from(node.node_id), &node.text)?;
            let path = json(
                &node.path,
                if limits.max_depth == isize::MAX as usize {
                    isize::MAX as usize
                } else {
                    HEADER_MAX
                },
            )?;
            nodes.push(Node {
                id: node.node_id,
                bytes: node.text.len(),
                checksum: text_checksum(&path),
                path,
            });
        }
        let mut mappings = Vec::with_capacity(extracted.mappings.len());
        for mapping in &extracted.mappings {
            if cancellation.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(error(ErrorKind::Cancelled, "text preparation cancelled"));
            }
            let payload = json(mapping, CHUNK)?;
            mappings.push(Mapping {
                start: mapping.start,
                end: mapping.end,
                checksum: text_checksum(&payload),
                payload,
            });
        }
        Ok(Self {
            dataset: input.dataset,
            extractor: json(
                &input.extractor,
                if limits.max_text_bytes == isize::MAX as usize {
                    isize::MAX as usize
                } else {
                    HEADER_MAX
                },
            )?,
            header,
            chunks,
            nodes,
            mappings,
        })
    }
}
