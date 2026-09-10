use super::*;
pub(super) const NAMES: &[&str] = &[
    "lugus_create_binding",
    "lugus_read_binding",
    "lugus_list_bindings",
    "lugus_revoke_binding",
    "lugus_binding_history",
    "lugus_fetch_bound_prices",
];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    request: BindRequest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    binding_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    page: PageRequest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    binding_id: String,
    page: PageRequest,
}
fn decode<T: serde::de::DeserializeOwned>(call: &ToolCall) -> Result<T> {
    serde_json::from_value(call.arguments.clone())
        .map_err(|_| invalid("invalid binding tool arguments"))
}
impl ResearchExecutor {
    pub(super) async fn binding_call(
        &self,
        scope: &Scope,
        call: &ToolCall,
    ) -> Result<(bool, Value)> {
        let app = &self.application;
        let result = match call.name.as_str() {
            "lugus_create_binding" => self.value(
                app.create_binding(scope, &decode::<Create>(call)?.request)
                    .await?,
            )?,
            "lugus_read_binding" => self.value(
                app.read_binding(scope, &decode::<Read>(call)?.binding_id)
                    .await?,
            )?,
            "lugus_list_bindings" => {
                self.value(app.list_bindings(scope, decode::<List>(call)?.page).await?)?
            }
            "lugus_revoke_binding" => self.value(
                app.revoke_binding(scope, &decode::<RevokeBindingRequest>(call)?)
                    .await?,
            )?,
            "lugus_binding_history" => {
                let args: History = decode(call)?;
                self.value(
                    app.binding_history(scope, &args.binding_id, args.page)
                        .await?,
                )?
            }
            "lugus_fetch_bound_prices" => {
                let receipt = app
                    .fetch_bound_prices(scope, &self.offering, &decode::<BoundPriceRequest>(call)?)
                    .await?;
                return self.await_job(scope, receipt).await;
            }
            _ => return Err(invalid("unknown binding tool")),
        };
        Ok((true, result))
    }
}
pub(super) fn tool_specs(limits: &Limits, offering: &Offering) -> Vec<ToolSpec> {
    let identifier = || {
        object(vec![
            (
                "namespace",
                json!({"type":"string","minLength":1,"maxLength":128}),
            ),
            (
                "value",
                json!({"type":"string","minLength":1,"maxLength":128}),
            ),
        ])
    };
    let page = || {
        object(vec![
            ("offset", integer(usize::MAX)),
            (
                "limit",
                json!({"type":"integer","minimum":1,"maximum":limits.max_read_page_items}),
            ),
        ])
    };
    let mut create = object(vec![
        ("company_dataset_id", id()),
        (
            "company_observation_id",
            json!({"type":"integer","minimum":1,"maximum":i64::MAX}),
        ),
        (
            "listing",
            object(vec![
                ("ticker", identifier()),
                ("exchange", json!({"oneOf":[identifier(),{"type":"null"}]})),
            ]),
        ),
        ("instrument_fetch_id", id()),
        ("supersedes", json!({"oneOf":[id(),{"type":"null"}]})),
    ]);
    create["required"] = json!([
        "company_dataset_id",
        "company_observation_id",
        "listing",
        "instrument_fetch_id"
    ]);
    let mut specs: Vec<_> = vec![
        ("create_binding","Create a source-supported binding from exact saved company/listing and instrument evidence; no verification authority comes from arguments.",object(vec![("request",create)])),
        ("read_binding","Read immutable binding evidence and current status offline.",object(vec![("binding_id",id())])),
        ("list_bindings","List bounded binding revisions and statuses offline; no default listing is implied.",object(vec![("page",page())])),
        ("revoke_binding","Revoke an owned binding for future preparations; retain its history.",object(vec![("binding_id",id())])),
        ("binding_history","Read bounded immutable status history offline.",object(vec![("binding_id",id()),("page",page())])),
    ].into_iter().map(|(name,description,input_schema)| ToolSpec {name:format!("lugus_{name}"),description:description.into(),input_schema}).collect();
    if offering
        .operations()
        .any(|o| o.operation == Operation::Prices)
    {
        specs.push(ToolSpec {name:"lugus_fetch_bound_prices".into(), description:"Fetch prices for one explicit active binding using its pinned provider and native instrument.".into(),input_schema:object(vec![("binding_id",id()),("start",json!({"type":"string","format":"date"})),("end",json!({"type":"string","format":"date"})),("page_size",json!({"type":"integer","minimum":1,"maximum":1000}))])});
    }
    specs
}
