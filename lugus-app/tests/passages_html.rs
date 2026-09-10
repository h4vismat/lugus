use lugus_app::ErrorKind;
use lugus_app::passages::*;
use std::sync::atomic::AtomicBool;

fn extract(bytes: &[u8]) -> ExtractedText {
    extract_html(
        bytes,
        "text/html",
        &TextLimits::default(),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn readable_filing_retains_visible_numbers_and_source_offsets() {
    let extracted = extract(include_bytes!("fixtures/filing.html"));
    assert_eq!(
        extracted.text,
        "Risk factors\nRevenue & cash grew.\nSão Paulo: (1,234.50) USD\nYear\tRevenue\n2025\t1,234.50"
    );
    let page = text_page(
        &extracted.text,
        "representation",
        13,
        33,
        &TextLimits::default(),
    )
    .unwrap();
    assert_eq!(page.text, "Revenue & cash grew.");
    assert!(validate_selection(&extracted.text, 1, 2, "wrong").is_err());
    let mappings = clip_mappings(&extracted.mappings, 13, 33).unwrap();
    let sources = resolve_sources(&extracted.source_nodes, &mappings, 65536).unwrap();
    assert_eq!(
        sources.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
        ["Revenue & cash ", "grew", "."]
    );
    assert_eq!(
        sources.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(),
        [(0, 15), (0, 4), (0, 1)]
    );
    assert_eq!(extracted.extractor.policy, "html-text-v1");
    assert_eq!(extracted.decoder, "utf-8");
    assert!(!extracted.limitations.is_empty());
}

#[test]
fn canonical_ranges_are_utf8_exact_and_normalized_sources_retain_whitespace() {
    let e = extract("<p>é a \n\t <b> b</b> a</p>".as_bytes());
    assert_eq!(e.text, "é a b a");
    assert!(validate_selection(&e.text, 1, 2, "é").is_err());
    assert!(validate_selection(&e.text, 0, 0, "").is_err());
    assert!(validate_selection(&e.text, 0, usize::MAX, "é").is_err());
    validate_selection(&e.text, 3, 4, "a").unwrap();
    validate_selection(&e.text, 7, 8, "a").unwrap();
    let m = clip_mappings(&e.mappings, 4, 5).unwrap();
    assert!(m.iter().all(|m| m.kind == MappingKind::Normalized));
    let s = resolve_sources(&e.source_nodes, &m, 65536).unwrap();
    assert_eq!(
        s.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
        [" \n\t ", " "]
    );
    let e = extract(b"<p>one</p><p>two</p>");
    assert!(clip_mappings(&e.mappings, 3, 4).is_err());
}

#[test]
fn decoding_is_strict_and_explicit() {
    let l = TextLimits::default();
    let c = AtomicBool::new(false);
    assert_eq!(
        extract_html(b"<p>\x80</p>", "text/html; charset=windows-1252", &l, &c)
            .unwrap()
            .text,
        "€"
    );
    assert_eq!(
        extract(b"<meta charset='windows-1252'><p>\x80</p>").decoder,
        "windows-1252"
    );
    assert_eq!(extract(b"\xef\xbb\xbf<p>ok</p>").text, "ok");
    for (b, media) in [
        (&b"\xff"[..], "text/html"),
        (&b"<p>ok</p>"[..], "text/html; charset=utf-16"),
        (
            &b"\xef\xbb\xbf<p>ok</p>"[..],
            "text/html; charset=windows-1252",
        ),
        (
            &b"<meta charset=windows-1252>"[..],
            "text/html; charset=utf-8",
        ),
    ] {
        assert!(extract_html(b, media, &l, &c).is_err());
    }
    for media in [
        "application/pdf",
        "text/plain",
        "application/xml",
        "application/xhtml+xml",
    ] {
        assert_eq!(
            extract_html(b"<p>ok</p>", media, &l, &c).unwrap_err().kind,
            ErrorKind::Unsupported
        );
    }
}

#[test]
fn bounds_and_cancellation_fail_without_partial_output() {
    let c = AtomicBool::new(false);
    for limits in [
        TextLimits {
            max_input_bytes: 4,
            ..Default::default()
        },
        TextLimits {
            max_text_bytes: 2,
            ..Default::default()
        },
        TextLimits {
            max_nodes: 3,
            ..Default::default()
        },
        TextLimits {
            max_depth: 3,
            ..Default::default()
        },
        TextLimits {
            max_mappings: 1,
            ..Default::default()
        },
        TextLimits {
            max_source_bytes: 2,
            ..Default::default()
        },
    ] {
        assert!(
            extract_html(
                b"<div><p>hello <b>world</b></p></div>",
                "text/html",
                &limits,
                &c
            )
            .is_err()
        );
    }
    assert_eq!(
        extract_html(
            b"<p>hello</p>",
            "text/html",
            &TextLimits::default(),
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .kind,
        ErrorKind::Cancelled
    );
    assert!(
        TextLimits {
            max_input_bytes: usize::MAX,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        TextLimits {
            max_page_bytes: 0,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        serde_json::from_str::<CreatePassageRequest>(
            r#"{"representation_id":"r","start":0,"end":1,"expected_text":"x","document":{}}"#
        )
        .is_err()
    );
    let deep = format!("{}x{}", "<div>".repeat(300), "</div>".repeat(300));
    assert!(extract_html(deep.as_bytes(), "text/html", &TextLimits::default(), &c).is_err());
}

#[test]
fn malformed_nesting_is_deterministic_and_inline_entity_offsets_are_decoded() {
    let e = extract(b"<p>A <b>B<i>C</b>D</i> &amp; E");
    assert_eq!(e.text, "A BCD & E");
    assert_eq!(e, extract(b"<p>A <b>B<i>C</b>D</i> &amp; E"));
    let e = extract(b"<p>A&amp;B</p>");
    let s = resolve_sources(
        &e.source_nodes,
        &clip_mappings(&e.mappings, 1, 2).unwrap(),
        4096,
    )
    .unwrap();
    assert_eq!((s[0].start, s[0].end, s[0].text.as_str()), (1, 2, "&"));
}

#[test]
fn pages_and_sources_are_bounded_including_serialization_escaping() {
    let e = extract(b"<p>abcdefghij</p>");
    let m = clip_mappings(&e.mappings, 2, 5).unwrap();
    let source = resolve_sources(&e.source_nodes, &m, 4096).unwrap();
    assert_eq!(source[0].text, "cde");
    assert_eq!((source[0].start, source[0].end), (2, 5));
    assert!(resolve_sources(&e.source_nodes, &m, 3).is_err());
    assert!(
        text_page(
            &e.text,
            "r",
            0,
            10,
            &TextLimits {
                max_page_bytes: 9,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(check_envelope(&"\"\"\"", 4).is_err());
    let request = CreatePassageRequest {
        representation_id: "r".into(),
        start: 100,
        end: 103,
        expected_text: "cde".into(),
    };
    assert_eq!(
        validate_selection_slice("cde", 100, &request, &TextLimits::default()).unwrap(),
        "cde"
    );
    assert!(
        validate_selection_slice(
            "cde",
            100,
            &request,
            &TextLimits {
                max_passage_bytes: 2,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn encoding_lookalikes_are_not_declarations_and_transport_duplicates_conflict() {
    assert_eq!(
        extract(b"<!-- <meta charset=utf-16> --><script>'<meta charset=utf-16>'</script><p>ok</p>")
            .text,
        "ok"
    );
    assert!(
        extract_html(
            b"<p>ok</p>",
            "text/html; charset=utf-8; charset=windows-1252",
            &TextLimits::default(),
            &AtomicBool::new(false)
        )
        .is_err()
    );
}

#[test]
fn cancellation_during_parsing_returns_a_normal_error() {
    let input = format!("<p>{}</p>", "a ".repeat(2_000_000));
    let cancelled = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(1));
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let error = extract_html(
            input.as_bytes(),
            "text/html",
            &TextLimits::default(),
            &cancelled,
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Cancelled);
    });
}

#[test]
fn replaceable_adapter_output_is_validated_before_persistence() {
    let limits = TextLimits::default();
    let cancel = AtomicBool::new(false);
    let original = extract(b"<p>one <b>two</b></p>");
    validate_extracted(&original, &limits, &cancel).unwrap();
    let mut forged = original.clone();
    forged.text_checksum = "wrong".into();
    assert!(validate_extracted(&forged, &limits, &cancel).is_err());
    let mut forged = original.clone();
    forged.mappings[0].source.as_mut().unwrap().end = 1000;
    assert!(validate_extracted(&forged, &limits, &cancel).is_err());
    let mut forged = original.clone();
    forged.mappings.remove(0);
    assert!(validate_extracted(&forged, &limits, &cancel).is_err());
    let mut forged = original.clone();
    forged.source_nodes[0].text = "bad quote".into();
    assert!(validate_extracted(&forged, &limits, &cancel).is_err());
    let mut forged = original.clone();
    forged.source_nodes.push(forged.source_nodes[0].clone());
    assert!(validate_extracted(&forged, &limits, &cancel).is_err());
}

#[test]
fn passage_building_pins_trusted_provenance_and_rejects_cross_workspace() {
    use lugus_app::{DatasetHeader, DatasetKind, DatasetProjection, ProviderIdentity, Scope};
    let now = "2026-09-10T00:00:00Z".parse().unwrap();
    let provider = ProviderIdentity {
        instance_id: "sec".into(),
        plugin_id: "sec".into(),
        plugin_version: "1".into(),
    };
    let dataset = DatasetHeader {
        binding_id: None,
        id: "dataset".into(),
        workspace_id: "workspace".into(),
        repository_id: "repository".into(),
        provider: provider.clone(),
        fetch_id: "fetch".into(),
        kind: DatasetKind::Document,
        projection: DatasetProjection::Document,
        query: serde_json::json!({}),
        created_at: now,
        row_count: 1,
        policy: None,
        selected_run: None,
        coverage: None,
        limitations: vec![],
        conflicts: vec![],
        resolution_status: None,
        source_snapshot: None,
        source_coverage: None,
        document: Some(lugus_financial::storage::DocumentObservation {
            provider,
            checksum: "original-checksum".into(),
            source_url: "https://example.test/original".into(),
            media_type: "text/html".into(),
            retrieved_at: now,
        }),
        error: None,
    };
    let e = extract(b"<p>one <b>two</b></p>");
    let representation = build_representation("representation".into(), &dataset, &e, now).unwrap();
    assert_eq!(representation.document.checksum, "original-checksum");
    let scope = Scope {
        workspace_id: "workspace".into(),
        request_id: "request".into(),
        run_id: None,
    };
    let request = CreatePassageRequest {
        representation_id: "representation".into(),
        start: 4,
        end: 7,
        expected_text: "two".into(),
    };
    let passage = build_passage(
        "passage".into(),
        scope.clone(),
        representation.clone(),
        &request,
        "two",
        4,
        &e.mappings,
        now,
        &TextLimits::default(),
        65536,
    )
    .unwrap();
    assert_eq!(passage.quote, "two");
    assert_eq!(passage.quote_checksum, text_checksum("two"));
    assert_eq!(
        passage.representation.document.source_url,
        "https://example.test/original"
    );
    assert!(
        build_passage(
            "passage".into(),
            scope.clone(),
            representation.clone(),
            &request,
            "two",
            4,
            &e.mappings,
            now,
            &TextLimits::default(),
            4
        )
        .is_err()
    );
    let wrong_scope = Scope {
        workspace_id: "another".into(),
        ..scope
    };
    assert_eq!(
        build_passage(
            "passage".into(),
            wrong_scope,
            representation,
            &request,
            "two",
            4,
            &e.mappings,
            now,
            &TextLimits::default(),
            65536
        )
        .unwrap_err()
        .kind,
        ErrorKind::ScopeMismatch
    );
}

#[test]
fn hidden_template_fragments_cannot_bypass_the_depth_budget() {
    let input = format!(
        "{}hidden{}",
        "<template>".repeat(300),
        "</template>".repeat(300)
    );
    assert_eq!(
        extract_html(
            input.as_bytes(),
            "text/html",
            &TextLimits::default(),
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
}
