//! Scoped binding operations share the host's blocking store and supervised admission.
use super::*;
use chrono::NaiveDate;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundPriceRequest {
    pub binding_id: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub page_size: usize,
}
impl Application {
    pub async fn create_binding(
        &self,
        scope: &Scope,
        request: &BindRequest,
    ) -> Result<BindingRecord> {
        scope.validate()?;
        crate::agent_contract::check_serialized_size(request, self.inner.limits.max_input_bytes)?;
        validate_id(&request.company_dataset_id)?;
        validate_id(&request.instrument_fetch_id)?;
        if let Some(id) = &request.supersedes {
            validate_id(id)?;
        }
        let scope = scope.clone();
        let request = request.clone();
        self.store(move |s| s.bind(&scope, &request)).await
    }
    pub async fn read_binding(&self, scope: &Scope, id: &str) -> Result<BindingView> {
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_string();
        self.store(move |s| s.read_binding(&scope, &id)).await
    }
    pub async fn list_bindings(&self, scope: &Scope, page: PageRequest) -> Result<BindingPage> {
        scope.validate()?;
        let scope = scope.clone();
        self.store(move |s| s.list_bindings(&scope, page)).await
    }
    pub async fn revoke_binding(
        &self,
        scope: &Scope,
        request: &RevokeBindingRequest,
    ) -> Result<BindingView> {
        scope.validate()?;
        validate_id(&request.binding_id)?;
        crate::agent_contract::check_serialized_size(request, self.inner.limits.max_input_bytes)?;
        let scope = scope.clone();
        let request = request.clone();
        self.store(move |s| s.revoke_binding(&scope, &request))
            .await
    }
    pub async fn binding_history(
        &self,
        scope: &Scope,
        id: &str,
        page: PageRequest,
    ) -> Result<BindingHistoryPage> {
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_string();
        self.store(move |s| s.binding_history(&scope, &id, page))
            .await
    }
    pub async fn fetch_bound_prices_manual(
        &self,
        scope: &Scope,
        request: &BoundPriceRequest,
    ) -> Result<JobReceipt> {
        self.fetch_bound_prices(scope, &self.offering()?, request)
            .await
    }
    /// Use the turn's captured offering. Neither provider nor instrument comes from the caller.
    pub async fn fetch_bound_prices(
        &self,
        scope: &Scope,
        offering: &Offering,
        request: &BoundPriceRequest,
    ) -> Result<JobReceipt> {
        scope.validate()?;
        validate_id(&request.binding_id)?;
        crate::agent_contract::check_serialized_size(request, self.inner.limits.max_input_bytes)?;
        if request.start > request.end || !(1..=1000).contains(&request.page_size) {
            return Err(error(
                ErrorKind::InvalidInput,
                "invalid bound price range or page size",
            ));
        }
        let owned_scope = scope.clone();
        let id = request.binding_id.clone();
        let binding = self
            .store(move |s| s.prepare_binding(&owned_scope, &id))
            .await?;
        let pinned = &binding.instrument.provider;
        let offered = offering
            .operations()
            .find(|o| {
                o.operation == Operation::Prices && o.identity.instance_id == pinned.instance_id
            })
            .ok_or_else(|| {
                error(
                    ErrorKind::Unsupported,
                    "bound provider prices were not offered for this turn",
                )
            })?;
        if &offered.identity != pinned {
            return Err(error(
                ErrorKind::StaleReference,
                "bound provider identity changed; new evidence and binding required",
            ));
        }
        let command = FetchCommand::Prices {
            instance_id: pinned.instance_id.clone(),
            query: PriceQuery {
                instrument: binding.instrument.request.instrument.clone(),
                start: request.start,
                end: request.end,
                page_size: request.page_size,
                cursor: None,
            },
        };
        self.submit_prepared(scope, offering, command, Some(binding))
    }
}
