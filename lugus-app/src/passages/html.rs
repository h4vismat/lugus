use super::{domain::*, mapping::*};
use crate::{AppError, ErrorKind, Result};
use html5ever::{
    Attribute, QualName, parse_document,
    tendril::{StrTendril, TendrilSink},
    tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink},
};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::atomic::AtomicBool,
};

#[derive(Debug, Default)]
pub struct HtmlTextExtractor;
impl TextExtractor for HtmlTextExtractor {
    fn identity(&self) -> ExtractorIdentity {
        ExtractorIdentity::html_v1()
    }
    fn extract(
        &self,
        bytes: &[u8],
        media_type: &str,
        limits: &TextLimits,
        cancellation: &AtomicBool,
    ) -> Result<ExtractedText> {
        extract_html(bytes, media_type, limits, cancellation)
    }
}

/// Offline structural HTML extraction; no script execution or external resource loading.
pub fn extract_html(
    bytes: &[u8],
    media_type: &str,
    limits: &TextLimits,
    cancellation: &AtomicBool,
) -> Result<ExtractedText> {
    limits.validate()?;
    check_cancel(cancellation)?;
    if bytes.len() > limits.max_input_bytes {
        return Err(limit());
    }
    if !media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("text/html")
    {
        return Err(unsupported(
            "document media type is not supported for text extraction",
        ));
    }
    // TreeSink is infallible. Private typed unwinds stop allocation immediately, without
    // invoking panic hooks. Catch only our sentinel; programming panics remain panics.
    match catch_unwind(AssertUnwindSafe(|| {
        extract_inner(bytes, media_type, limits, cancellation)
    })) {
        Ok(result) => result,
        Err(payload) => match payload.downcast::<Abort>() {
            Ok(abort) => Err(abort.0),
            Err(payload) => resume_unwind(payload),
        },
    }
}
fn extract_inner(
    bytes: &[u8],
    media: &str,
    limits: &TextLimits,
    cancel: &AtomicBool,
) -> Result<ExtractedText> {
    let transport = encoding_parameters(media)?;
    let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        return Err(unsupported("unsupported HTML encoding"));
    }
    let bytes = if bom { &bytes[3..] } else { bytes };
    // HTML's encoding declaration window is the first 1024 bytes. Use the same
    // maintained tree parser for sniffing; comments/script content are not declarations.
    let sniff = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
    let sniff_tree = parse(&sniff, limits, cancel)?;
    let declared = sniff_tree.encoding.get();
    let chosen = transport.or(declared).unwrap_or("utf-8");
    if (bom && chosen != "utf-8") || transport.zip(declared).is_some_and(|(a, b)| a != b) {
        return Err(invalid("HTML encoding declarations disagree"));
    }
    let encoding = if chosen == "utf-8" {
        encoding_rs::UTF_8
    } else {
        encoding_rs::WINDOWS_1252
    };
    let decoded = encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .ok_or_else(|| invalid("HTML bytes do not match declared encoding"))?;
    check_cancel(cancel)?;
    let tree = parse(&decoded, limits, cancel)?;
    if tree.encoding.get().is_some_and(|value| value != chosen) {
        return Err(invalid("HTML encoding declarations disagree"));
    }
    let mut emit = Emitter::new(limits, cancel);
    tree.walk(0, &mut Vec::new(), false, &mut emit)?;
    check_cancel(cancel)?;
    let text_checksum = text_checksum(&emit.text);
    let extracted = ExtractedText { extractor: ExtractorIdentity::html_v1(), decoder: chosen.into(), text: emit.text, text_checksum,
        source_nodes: emit.nodes, mappings: emit.mappings,
        limitations: vec!["Structural HTML text only; external/computed CSS, layout, pseudo-elements and script-generated content are not evaluated.".into(),
            "Source coordinates are UTF-8 offsets in entity-decoded parsed text nodes, not raw HTML or browser offsets.".into(),
            "Encoding declarations are selected from transport/BOM and the first 1024 bytes; later conflicting declarations fail.".into()] };
    validate_extracted(&extracted, limits, cancel)?;
    Ok(extracted)
}
fn unsupported(message: &'static str) -> AppError {
    AppError::new(ErrorKind::Unsupported, message, false)
}
fn encoding_label(label: &str) -> Result<&'static str> {
    match label.trim().to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" => Ok("utf-8"),
        "windows-1252" | "cp1252" => Ok("windows-1252"),
        _ => Err(unsupported("unsupported HTML encoding")),
    }
}
fn encoding_parameters(value: &str) -> Result<Option<&'static str>> {
    let mut selected = None;
    for part in value.split(';').skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("charset") {
            let encoding = encoding_label(value.trim().trim_matches(['\'', '"']))?;
            if selected.is_some_and(|previous| previous != encoding) {
                return Err(invalid("HTML encoding declarations disagree"));
            }
            selected = Some(encoding);
        }
    }
    Ok(selected)
}
#[derive(Debug)]
struct Abort(AppError);
fn abort(error: AppError) -> ! {
    resume_unwind(Box::new(Abort(error)))
}
fn checked<T>(result: Result<T>) -> T {
    result.unwrap_or_else(|error| abort(error))
}
#[derive(Debug, Clone)]
struct Handle {
    id: usize,
    name: QualName,
}
struct Node {
    handle: Handle,
    parent: Option<usize>,
    children: Vec<usize>,
    attrs: Vec<Attribute>,
    text: Option<String>,
    template: Option<Handle>,
    integration: bool,
}
struct BoundedTree<'a> {
    nodes: RefCell<Vec<Node>>,
    limits: &'a TextLimits,
    cancel: &'a AtomicBool,
    allocated_bytes: Cell<usize>,
    encoding: Cell<Option<&'static str>>,
}
impl<'a> BoundedTree<'a> {
    fn new(limits: &'a TextLimits, cancel: &'a AtomicBool) -> Self {
        Self {
            nodes: RefCell::new(vec![Node {
                handle: Handle {
                    id: 0,
                    name: QualName::new(None, html5ever::ns!(html), "document".into()),
                },
                parent: None,
                children: vec![],
                attrs: vec![],
                text: None,
                template: None,
                integration: false,
            }]),
            limits,
            cancel,
            allocated_bytes: Cell::new(0),
            encoding: Cell::new(None),
        }
    }
    fn check(&self) {
        checked(check_cancel(self.cancel));
    }
    fn charge(&self, bytes: usize) {
        self.check();
        let total = self
            .allocated_bytes
            .get()
            .checked_add(bytes)
            .filter(|n| *n <= self.limits.max_source_bytes)
            .unwrap_or_else(|| abort(limit()));
        self.allocated_bytes.set(total);
    }
    fn alloc(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        text: Option<String>,
        template: Option<Handle>,
        integration: bool,
    ) -> Handle {
        self.check();
        let mut nodes = self.nodes.borrow_mut();
        if nodes.len() >= self.limits.max_nodes {
            abort(limit());
        }
        let handle = Handle {
            id: nodes.len(),
            name,
        };
        nodes.push(Node {
            handle: handle.clone(),
            parent: None,
            children: vec![],
            attrs,
            text,
            template,
            integration,
        });
        handle
    }
    fn plain(&self, text: Option<String>) -> Handle {
        self.alloc(
            QualName::new(None, html5ever::ns!(html), "".into()),
            vec![],
            text,
            None,
            false,
        )
    }
    fn detach(&self, target: usize) {
        self.check();
        let mut nodes = self.nodes.borrow_mut();
        if let Some(parent) = nodes[target].parent.take() {
            nodes[parent].children.retain(|id| *id != target);
        }
    }
    fn attach(&self, parent: usize, before: Option<usize>, child: NodeOrText<Handle>) {
        self.check();
        let previous = {
            let nodes = self.nodes.borrow();
            let index = before
                .and_then(|id| nodes[parent].children.iter().position(|x| *x == id))
                .unwrap_or(nodes[parent].children.len());
            index.checked_sub(1).map(|i| nodes[parent].children[i])
        };
        let child = match child {
            NodeOrText::AppendNode(child) => child,
            NodeOrText::AppendText(text) => {
                self.charge(text.len());
                if let Some(previous) = previous
                    && let Some(existing) = self.nodes.borrow_mut()[previous].text.as_mut()
                {
                    existing.push_str(&text);
                    return;
                }
                self.plain(Some(text.to_string()))
            }
        };
        self.detach(child.id);
        {
            let nodes = self.nodes.borrow();
            let mut depth = 1;
            let mut ancestor = Some(parent);
            while let Some(id) = ancestor {
                self.check();
                if id == child.id || depth > self.limits.max_depth {
                    abort(limit());
                }
                depth += 1;
                ancestor = nodes[id].parent;
            }
            // Reparenting an existing subtree must also fit the destination depth.
            let mut stack = vec![(child.id, depth)];
            while let Some((id, level)) = stack.pop() {
                self.check();
                if level > self.limits.max_depth + 1 {
                    abort(limit());
                }
                stack.extend(nodes[id].children.iter().map(|id| (*id, level + 1)));
                if let Some(fragment) = &nodes[id].template {
                    stack.push((fragment.id, level + 1));
                }
            }
        }
        let mut nodes = self.nodes.borrow_mut();
        nodes[child.id].parent = Some(parent);
        let index = before
            .and_then(|id| nodes[parent].children.iter().position(|x| *x == id))
            .unwrap_or(nodes[parent].children.len());
        nodes[parent].children.insert(index, child.id);
    }
    fn note_encoding(&self, attrs: &[Attribute]) {
        let attr = |name: &str| {
            attrs
                .iter()
                .find(|a| a.name.local.as_ref() == name)
                .map(|a| a.value.as_ref())
        };
        let declared = attr("charset").map(encoding_label).transpose();
        let mut labels = vec![checked(declared)];
        if attr("http-equiv").is_some_and(|v| v.eq_ignore_ascii_case("content-type"))
            && let Some(content) = attr("content")
        {
            labels.push(checked(encoding_parameters(content)));
        }
        for value in labels.into_iter().flatten() {
            if self
                .encoding
                .get()
                .is_some_and(|previous| previous != value)
            {
                abort(invalid("HTML encoding declarations disagree"));
            }
            self.encoding.set(Some(value));
        }
    }
    fn walk(
        &self,
        id: usize,
        path: &mut Vec<u32>,
        hidden: bool,
        out: &mut Emitter<'_>,
    ) -> Result<()> {
        check_cancel(self.cancel)?;
        let nodes = self.nodes.borrow();
        let node = &nodes[id];
        let name = node.handle.name.local.as_ref();
        let hidden = hidden
            || matches!(name, "head" | "script" | "style" | "template" | "ix:hidden")
            || hidden_attrs(&node.attrs);
        if hidden {
            return Ok(());
        }
        if path.len() > self.limits.max_depth {
            return Err(limit());
        }
        let separator = separator(name);
        if let Some(separator) = separator {
            out.separator(separator);
        }
        if let Some(text) = &node.text {
            out.node(id as u32, path, text)?;
        }
        for (ordinal, child) in node.children.iter().enumerate() {
            path.push(ordinal as u32);
            self.walk(*child, path, hidden, out)?;
            path.pop();
        }
        if let Some(separator) = separator {
            out.separator(separator);
        }
        Ok(())
    }
}
impl TreeSink for BoundedTree<'_> {
    type Handle = Handle;
    type Output = Self;
    type ElemName<'a>
        = &'a QualName
    where
        Self: 'a;
    fn finish(self) -> Self {
        self
    }
    fn parse_error(&self, _: Cow<'static, str>) {
        self.check();
    }
    fn get_document(&self) -> Handle {
        self.nodes.borrow()[0].handle.clone()
    }
    fn elem_name<'a>(&'a self, target: &'a Handle) -> &'a QualName {
        &target.name
    }
    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        self.charge(name.local.len());
        for attr in &attrs {
            self.charge(
                attr.name
                    .local
                    .len()
                    .checked_add(attr.value.len())
                    .unwrap_or_else(|| abort(limit())),
            );
        }
        if name.local.as_ref() == "meta" {
            self.note_encoding(&attrs);
        }
        let template = flags.template.then(|| self.plain(None));
        let element = self.alloc(
            name,
            attrs,
            None,
            template.clone(),
            flags.mathml_annotation_xml_integration_point,
        );
        // Template fragments are not ordinary DOM children, but belong to the
        // same resource-depth tree. Keep that ownership edge for depth checks.
        if let Some(fragment) = template {
            self.nodes.borrow_mut()[fragment.id].parent = Some(element.id);
        }
        element
    }
    fn create_comment(&self, _: StrTendril) -> Handle {
        self.plain(None)
    }
    fn create_pi(&self, _: StrTendril, _: StrTendril) -> Handle {
        self.plain(None)
    }
    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        self.attach(parent.id, None, child);
    }
    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        previous: &Handle,
        child: NodeOrText<Handle>,
    ) {
        let parent = self.nodes.borrow()[element.id].parent;
        if let Some(parent) = parent {
            self.attach(parent, Some(element.id), child);
        } else {
            self.append(previous, child);
        }
    }
    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {
        let node = self.plain(None);
        self.attach(0, None, NodeOrText::AppendNode(node));
    }
    fn get_template_contents(&self, target: &Handle) -> Handle {
        self.nodes.borrow()[target.id]
            .template
            .as_ref()
            .expect("parser template invariant")
            .clone()
    }
    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        x.id == y.id
    }
    fn set_quirks_mode(&self, _: QuirksMode) {}
    fn append_before_sibling(&self, sibling: &Handle, child: NodeOrText<Handle>) {
        let parent = self.nodes.borrow()[sibling.id].parent;
        if let Some(parent) = parent {
            self.attach(parent, Some(sibling.id), child);
        }
    }
    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        for attr in attrs {
            self.check();
            let exists = self.nodes.borrow()[target.id]
                .attrs
                .iter()
                .any(|a| a.name == attr.name);
            if !exists {
                self.charge(
                    attr.name
                        .local
                        .len()
                        .checked_add(attr.value.len())
                        .unwrap_or_else(|| abort(limit())),
                );
                self.nodes.borrow_mut()[target.id].attrs.push(attr);
            }
        }
    }
    fn remove_from_parent(&self, target: &Handle) {
        self.detach(target.id);
    }
    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let children = self.nodes.borrow()[node.id].children.clone();
        for id in children {
            let handle = self.nodes.borrow()[id].handle.clone();
            self.attach(new_parent.id, None, NodeOrText::AppendNode(handle));
        }
    }
    fn is_mathml_annotation_xml_integration_point(&self, target: &Handle) -> bool {
        self.nodes.borrow()[target.id].integration
    }
}
fn parse<'a>(
    text: &str,
    limits: &'a TextLimits,
    cancel: &'a AtomicBool,
) -> Result<BoundedTree<'a>> {
    let mut parser = parse_document(BoundedTree::new(limits, cancel), Default::default());
    let mut offset = 0;
    while offset < text.len() {
        check_cancel(cancel)?;
        let mut end = (offset + 4096).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        parser.process(StrTendril::from_slice(&text[offset..end]));
        offset = end;
    }
    check_cancel(cancel)?;
    Ok(parser.finish())
}
fn hidden_attrs(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if attr.name.local.as_ref() == "hidden" {
            return true;
        }
        if attr.name.local.as_ref() != "style" {
            return false;
        }
        attr.value.split(';').any(|declaration| {
            let Some((key, value)) = declaration.split_once(':') else {
                return false;
            };
            let value = value.split('!').next().unwrap_or_default().trim();
            (key.trim().eq_ignore_ascii_case("display") && value.eq_ignore_ascii_case("none"))
                || (key.trim().eq_ignore_ascii_case("visibility")
                    && value.eq_ignore_ascii_case("hidden"))
        })
    })
}
fn separator(name: &str) -> Option<char> {
    match name {
        "td" | "th" => Some('\t'),
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "div" | "section" | "article"
        | "header" | "footer" | "li" | "ul" | "ol" | "tr" | "table" | "br" | "hr"
        | "blockquote" | "pre" => Some('\n'),
        _ => None,
    }
}

struct Emitter<'a> {
    limits: &'a TextLimits,
    cancel: &'a AtomicBool,
    text: String,
    nodes: Vec<SourceNode>,
    mappings: Vec<SourceMapping>,
    pending: Vec<SourceInterval>,
    pending_exact: bool,
    structural: Option<char>,
    source_bytes: usize,
}
impl<'a> Emitter<'a> {
    fn new(limits: &'a TextLimits, cancel: &'a AtomicBool) -> Self {
        Self {
            limits,
            cancel,
            text: String::new(),
            nodes: vec![],
            mappings: vec![],
            pending: vec![],
            pending_exact: false,
            structural: None,
            source_bytes: 0,
        }
    }
    fn separator(&mut self, separator: char) {
        self.pending.clear();
        if self.structural != Some('\n') {
            self.structural = Some(separator);
        }
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.source_bytes = self
            .source_bytes
            .checked_add(bytes)
            .filter(|n| *n <= self.limits.max_source_bytes)
            .ok_or_else(limit)?;
        Ok(())
    }
    fn mapping(&mut self, mapping: SourceMapping) -> Result<()> {
        if let Some(previous) = self.mappings.last_mut()
            && previous.kind == MappingKind::Exact
            && mapping.kind == MappingKind::Exact
            && previous.end == mapping.start
            && let (Some(a), Some(b)) = (&mut previous.source, &mapping.source)
            && a.node_id == b.node_id
            && a.end == b.start
        {
            a.end = b.end;
            previous.end = mapping.end;
            return Ok(());
        }
        if self.mappings.len() >= self.limits.max_mappings {
            return Err(limit());
        }
        // Reserve decimal digit growth when exact runs coalesce later.
        self.charge(check_envelope(&mapping, self.limits.max_source_bytes)? + 81)?;
        self.mappings.push(mapping);
        Ok(())
    }
    fn push(&mut self, value: char) -> Result<(usize, usize)> {
        check_cancel(self.cancel)?;
        let start = self.text.len();
        let end = start
            .checked_add(value.len_utf8())
            .filter(|n| *n <= self.limits.max_text_bytes)
            .ok_or_else(limit)?;
        self.text.push(value);
        Ok((start, end))
    }
    fn node(&mut self, node_id: u32, path: &[u32], text: &str) -> Result<()> {
        let node = SourceNode {
            node_id,
            path: path.into(),
            text: text.into(),
        };
        self.charge(check_envelope(&node, self.limits.max_source_bytes)? + 1)?;
        self.nodes.push(node);
        for (offset, character) in text.char_indices() {
            check_cancel(self.cancel)?;
            let source = SourceInterval {
                node_id,
                start: offset,
                end: offset + character.len_utf8(),
            };
            if character.is_whitespace() {
                if self.text.is_empty() || self.structural.is_some() {
                    continue;
                }
                self.pending_exact = self.pending.is_empty() && character == ' ';
                if let Some(previous) = self.pending.last_mut()
                    && previous.node_id == node_id
                    && previous.end == offset
                {
                    previous.end = source.end;
                    continue;
                }
                if self.pending.len() >= self.limits.max_mappings {
                    return Err(limit());
                }
                self.pending.push(source);
                continue;
            }
            if !self.text.is_empty() {
                if let Some(separator) = self.structural.take() {
                    let (start, end) = self.push(separator)?;
                    self.mapping(SourceMapping {
                        start,
                        end,
                        kind: MappingKind::Synthetic,
                        source: None,
                    })?;
                } else if !self.pending.is_empty() {
                    let (start, end) = self.push(' ')?;
                    let kind = if self.pending_exact {
                        MappingKind::Exact
                    } else {
                        MappingKind::Normalized
                    };
                    for source in std::mem::take(&mut self.pending) {
                        self.mapping(SourceMapping {
                            start,
                            end,
                            kind,
                            source: Some(source),
                        })?;
                    }
                }
            } else {
                self.structural = None;
                self.pending.clear();
            }
            let (start, end) = self.push(character)?;
            self.mapping(SourceMapping {
                start,
                end,
                kind: MappingKind::Exact,
                source: Some(source),
            })?;
        }
        Ok(())
    }
}
