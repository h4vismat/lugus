use crate::{
    domain::*,
    error::{Error, ErrorKind, Result},
    selection::Evidence,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevenueBasis {
    ContractRevenueExcludingTax,
    Revenues,
}
impl RevenueBasis {
    pub fn concept(self) -> &'static str {
        match self {
            Self::ContractRevenueExcludingTax => {
                "RevenueFromContractWithCustomerExcludingAssessedTax"
            }
            Self::Revenues => "Revenues",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnualPolicy {
    pub period_end: NaiveDate,
    pub years: u8,
    pub revenue_basis: RevenueBasis,
}
impl AnnualPolicy {
    pub fn validate(&self) -> Result<()> {
        if !(1..=5).contains(&self.years) {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "annual period count must be between one and five",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactInput {
    pub dataset_id: String,
    pub ordinal: usize,
    pub evidence: Evidence<Fact>,
}
impl FactInput {
    pub fn reference(&self) -> InputRef {
        InputRef {
            dataset_id: self.dataset_id.clone(),
            ordinal: self.ordinal,
            observation_id: self.evidence.retrieval.observation_id.to_string(),
            fingerprint: self.evidence.retrieval.fingerprint.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InputRef {
    pub dataset_id: String,
    pub ordinal: usize,
    pub observation_id: String,
    pub fingerprint: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonIssue {
    pub code: String,
    pub detail: String,
    pub inputs: Vec<InputRef>,
}
impl ComparisonIssue {
    pub fn new(code: &str, detail: &str, mut inputs: Vec<InputRef>) -> Self {
        inputs.sort();
        inputs.dedup();
        Self {
            code: code.into(),
            detail: detail.into(),
            inputs,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedAmount {
    pub value: Option<Decimal>,
    pub inputs: Vec<InputRef>,
    pub filing_ids: Vec<String>,
    pub issues: Vec<ComparisonIssue>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnualPeriod {
    pub start: Option<NaiveDate>,
    pub end: NaiveDate,
    pub days: Option<u16>,
    pub candidate_periods: Vec<Period>,
    pub revenue: SelectedAmount,
    pub net_income: SelectedAmount,
    pub issues: Vec<ComparisonIssue>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnualSelection {
    pub company: CompanyId,
    pub periods: Vec<AnnualPeriod>,
    pub issues: Vec<ComparisonIssue>,
}
