#![allow(dead_code)]
use lugus_financial::{
    comparison::*, domain::*, selection::Evidence, storage::ObservationRetrieval,
};
pub fn company() -> CompanyId {
    CompanyId {
        namespace: "sec:cik".into(),
        value: "0000000001".into(),
    }
}
pub fn policy() -> AnnualPolicy {
    AnnualPolicy {
        period_end: "2024-12-31".parse().unwrap(),
        years: 3,
        revenue_basis: RevenueBasis::ContractRevenueExcludingTax,
    }
}
pub fn fact(id: i64, year: i32, concept: &str, value: &str) -> FactInput {
    let time = "2025-02-01T00:00:00Z".parse().unwrap();
    let value = Fact {
        company: company(),
        namespace: "us-gaap".into(),
        concept: concept.into(),
        label: None,
        value: Decimal::new(value).unwrap(),
        unit: "USD".into(),
        period: Period::Duration {
            start: format!("{year}-01-01").parse().unwrap(),
            end: format!("{year}-12-31").parse().unwrap(),
        },
        filing_id: format!("filing-{year}"),
        form: "10-K".into(),
        filed: "2025-02-01".parse().unwrap(),
        fiscal_year: Some(2024),
        fiscal_period: Some("FY".into()),
        source_url: "https://fixture.test/filing".into(),
        retrieved_at: time,
    };
    FactInput {
        dataset_id: "dataset".into(),
        ordinal: id as usize,
        evidence: Evidence {
            retrieval: ObservationRetrieval {
                observation_id: id,
                kind: "fact".into(),
                fingerprint: value.fingerprint().unwrap(),
                retrieved_at: time,
            },
            value,
        },
    }
}
pub fn revenue(id: i64, year: i32, value: &str) -> FactInput {
    fact(
        id,
        year,
        "RevenueFromContractWithCustomerExcludingAssessedTax",
        value,
    )
}
pub fn selected(facts: &[FactInput]) -> AnnualSelection {
    select_annual(&company(), facts, &policy()).unwrap()
}
