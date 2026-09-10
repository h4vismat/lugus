use lugus_app::{
    AppError, Catalog, ErrorKind, FetchCommand, Limits, Operation, ProviderEntry, Scope,
};
use lugus_financial::domain::ProviderIdentity;
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};

fn identity(instance_id: &str, plugin_version: &str) -> ProviderIdentity {
    ProviderIdentity {
        instance_id: instance_id.into(),
        plugin_id: "fixture".into(),
        plugin_version: plugin_version.into(),
    }
}

fn capabilities(entries: &[(&str, u32)]) -> BTreeMap<String, u32> {
    entries
        .iter()
        .map(|(name, version)| ((*name).into(), *version))
        .collect()
}

fn entry(
    instance_id: &str,
    active: bool,
    available: bool,
    capabilities: &[(&str, u32)],
) -> ProviderEntry {
    ProviderEntry {
        identity: identity(instance_id, "1"),
        active,
        available,
        capabilities: self::capabilities(capabilities),
    }
}

fn prices(instance_id: &str) -> FetchCommand {
    serde_json::from_value(json!({
        "operation": "prices",
        "instance_id": instance_id,
        "query": {
            "instrument": {"namespace": "yahoo:symbol", "value": "ACME"},
            "start": "2025-01-01",
            "end": "2025-01-02",
            "cursor": null,
            "page_size": 100
        }
    }))
    .unwrap()
}

#[test]
fn duplicate_configured_instance_ids_are_rejected() {
    let error = Catalog::new(vec![
        entry("market-a", true, true, &[("market_data", 1)]),
        entry("market-a", false, false, &[]),
    ])
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Conflict);
}

#[test]
fn unsupported_capability_versions_are_preserved_but_not_offered() {
    let catalog = Catalog::new(vec![entry(
        "market-a",
        true,
        true,
        &[("market_data", 2), ("future_capability", 7)],
    )])
    .unwrap();
    let state = catalog.get("market-a").unwrap();
    assert_eq!(state.capabilities.get("market_data"), Some(&2));
    assert_eq!(state.capabilities.get("future_capability"), Some(&7));
    let offered = catalog.snapshot();
    assert!(offered.operations().next().is_none());
    assert_eq!(
        catalog
            .authorize(&offered, &prices("market-a"))
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
}

#[test]
fn every_fetch_requires_an_explicit_provider_instance() {
    let catalog = Catalog::new(vec![entry("market-a", true, true, &[("market_data", 1)])]).unwrap();
    let offered = catalog.snapshot();
    let error = catalog.authorize(&offered, &prices("")).unwrap_err();
    assert_eq!(error.kind, ErrorKind::AmbiguousProvider);
}

#[test]
fn restart_invalidates_an_existing_offering_generation() {
    let mut catalog =
        Catalog::new(vec![entry("market-a", true, true, &[("market_data", 1)])]).unwrap();
    let offered = catalog.snapshot();
    let generation = catalog.get("market-a").unwrap().generation;

    catalog
        .restart(
            identity("market-a", "2"),
            capabilities(&[("market_data", 1)]),
        )
        .unwrap();

    assert_eq!(catalog.get("market-a").unwrap().generation, generation + 1);
    assert_eq!(
        catalog
            .authorize(&offered, &prices("market-a"))
            .unwrap_err()
            .kind,
        ErrorKind::StaleReference
    );
}

#[test]
fn activation_during_a_turn_does_not_enlarge_its_offering() {
    let mut catalog =
        Catalog::new(vec![entry("market-a", false, true, &[("market_data", 1)])]).unwrap();
    let offered = catalog.snapshot();
    catalog.activate("market-a").unwrap();

    assert_eq!(
        catalog
            .authorize(&offered, &prices("market-a"))
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    assert!(
        catalog
            .authorize(&catalog.snapshot(), &prices("market-a"))
            .is_ok()
    );
}

#[test]
fn deactivation_after_offering_closes_dispatch() {
    let mut catalog =
        Catalog::new(vec![entry("market-a", true, true, &[("market_data", 1)])]).unwrap();
    let offered = catalog.snapshot();
    catalog.deactivate("market-a").unwrap();

    assert_eq!(
        catalog
            .authorize(&offered, &prices("market-a"))
            .unwrap_err()
            .kind,
        ErrorKind::Deactivated
    );
}

#[test]
fn unavailable_provider_is_not_offered_and_closes_an_existing_offering() {
    let mut catalog =
        Catalog::new(vec![entry("market-a", true, true, &[("market_data", 1)])]).unwrap();
    let offered = catalog.snapshot();
    catalog.set_available("market-a", false).unwrap();

    assert_eq!(
        catalog
            .authorize(&offered, &prices("market-a"))
            .unwrap_err()
            .kind,
        ErrorKind::Unavailable
    );
    assert!(catalog.snapshot().operations().next().is_none());
}

#[test]
fn offerings_map_only_the_supported_plugin_v1_capabilities() {
    let catalog = Catalog::new(vec![entry(
        "all",
        true,
        true,
        &[
            ("company_resolution", 1),
            ("filings", 1),
            ("fundamentals", 1),
            ("market_data", 1),
        ],
    )])
    .unwrap();
    let operations = catalog
        .snapshot()
        .operations()
        .map(|offered| offered.operation)
        .collect::<Vec<_>>();
    assert_eq!(
        operations,
        vec![
            Operation::Resolve,
            Operation::Lookup,
            Operation::Filings,
            Operation::Facts,
            Operation::Document,
            Operation::Prices,
        ]
    );
}

#[test]
fn scope_rejects_empty_oversized_and_control_character_ids() {
    let valid = Scope {
        workspace_id: "workspace".into(),
        request_id: "request".into(),
        run_id: Some("run".into()),
    };
    assert!(valid.validate().is_ok());

    for invalid in [
        Scope {
            workspace_id: "".into(),
            ..valid.clone()
        },
        Scope {
            request_id: "x".repeat(Scope::MAX_ID_BYTES + 1),
            ..valid.clone()
        },
        Scope {
            run_id: Some("\n".into()),
            ..valid.clone()
        },
    ] {
        assert_eq!(
            invalid.validate().unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }
}

#[test]
fn limits_reject_every_zero_bound() {
    let valid = Limits::default();
    assert!(valid.validate().is_ok());
    let invalid = [
        Limits {
            queue_capacity: 0,
            ..valid.clone()
        },
        Limits {
            max_concurrent_jobs: 0,
            ..valid.clone()
        },
        Limits {
            operation_timeout: Duration::ZERO,
            ..valid.clone()
        },
        Limits {
            max_pages_per_fetch: 0,
            ..valid.clone()
        },
        Limits {
            max_items_per_fetch: 0,
            ..valid.clone()
        },
        Limits {
            max_bytes_per_fetch: 0,
            ..valid.clone()
        },
        Limits {
            max_document_bytes: 0,
            ..valid.clone()
        },
        Limits {
            max_input_bytes: 0,
            ..valid.clone()
        },
        Limits {
            max_output_bytes: 0,
            ..valid.clone()
        },
        Limits {
            max_read_page_items: 0,
            ..valid.clone()
        },
        Limits {
            max_read_page_bytes: 0,
            ..valid
        },
    ];
    for limits in invalid {
        assert_eq!(limits.validate().unwrap_err().kind, ErrorKind::InvalidInput);
    }
}

#[test]
fn limits_reject_values_that_downstream_primitives_cannot_safely_bound() {
    let valid = Limits::default();
    let invalid = [
        Limits {
            queue_capacity: Limits::MAX_QUEUE_CAPACITY + 1,
            ..valid.clone()
        },
        Limits {
            max_concurrent_jobs: Limits::MAX_CONCURRENT_JOBS + 1,
            ..valid.clone()
        },
        Limits {
            operation_timeout: Limits::MAX_OPERATION_TIMEOUT + Duration::from_secs(1),
            ..valid.clone()
        },
        Limits {
            max_pages_per_fetch: Limits::MAX_PAGES_PER_FETCH + 1,
            ..valid.clone()
        },
        Limits {
            max_items_per_fetch: Limits::MAX_ITEMS_PER_FETCH + 1,
            ..valid.clone()
        },
        Limits {
            max_bytes_per_fetch: Limits::MAX_BYTES + 1,
            ..valid.clone()
        },
        Limits {
            max_output_bytes: Limits::MIN_OUTPUT_BYTES - 1,
            ..valid
        },
    ];
    for limits in invalid {
        assert_eq!(limits.validate().unwrap_err().kind, ErrorKind::InvalidInput);
    }
}

#[test]
fn root_fetch_queries_reject_continuation_cursors() {
    let mut filings: FetchCommand = serde_json::from_value(json!({
        "operation": "filings",
        "instance_id": "sec-a",
        "query": {
            "company": {"namespace": "sec:cik", "value": "0000000001"},
            "filed_from": "2025-01-01",
            "filed_to": "2025-01-02",
            "forms": [],
            "cursor": "next",
            "page_size": 100
        }
    }))
    .unwrap();
    assert_eq!(
        filings.validate().unwrap_err().kind,
        ErrorKind::InvalidInput
    );

    if let FetchCommand::Filings { query, .. } = &mut filings {
        query.cursor = None;
    }
    assert!(filings.validate().is_ok());

    let mut price_command = prices("market-a");
    if let FetchCommand::Prices { query, .. } = &mut price_command {
        query.cursor = Some("next".into());
    }
    assert_eq!(
        price_command.validate().unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn command_json_rejects_unknown_fields_at_every_application_boundary() {
    let cases = [
        json!({
            "operation": "resolve", "instance_id": "resolver-a", "input": "Acme",
            "unexpected": true
        }),
        json!({
            "operation": "filings", "instance_id": "sec-a",
            "query": {
                "company": {"namespace": "sec:cik", "value": "0000000001"},
                "filed_from": "2025-01-01", "filed_to": "2025-01-02",
                "forms": [], "cursor": null, "page_size": 100,
                "unexpected": true
            }
        }),
        json!({
            "operation": "lookup", "instance_id": "resolver-a",
            "request": {"identifier": {
                "namespace": "sec:cik", "value": "0000000001", "unexpected": true
            }}
        }),
        json!({
            "operation": "prices", "instance_id": "market-a",
            "query": {
                "instrument": {
                    "namespace": "yahoo:symbol", "value": "ACME", "unexpected": true
                },
                "start": "2025-01-01", "end": "2025-01-02",
                "cursor": null, "page_size": 100
            }
        }),
    ];
    for value in cases {
        assert!(serde_json::from_value::<FetchCommand>(value).is_err());
    }

    assert!(
        serde_json::from_value::<Scope>(json!({
            "workspace_id": "workspace", "request_id": "request", "run_id": null,
            "unexpected": true
        }))
        .is_err()
    );
}

#[test]
fn application_errors_bound_safe_messages() {
    let error = AppError::new(ErrorKind::Unavailable, "é".repeat(2_000), true);
    assert!(error.message.len() <= AppError::MAX_MESSAGE_BYTES);
    assert!(error.message.is_char_boundary(error.message.len()));
    assert!(error.retryable);
    assert_eq!(error.retry_after_seconds, None);
}
