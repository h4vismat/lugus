use crate::{Bridge, value};
use lugus_app::{comparison::ComparisonRequest, *};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    Start {
        conversation: String,
        request: ComparisonRequest,
    },
    Status {
        conversation: String,
        job: String,
    },
    Cancel {
        conversation: String,
        job: String,
    },
    Read {
        conversation: String,
        id: String,
    },
    List {
        conversation: String,
        #[serde(default)]
        offset: usize,
    },
    Rows {
        conversation: String,
        id: String,
        #[serde(default)]
        offset: usize,
    },
    Sources {
        conversation: String,
        id: String,
        #[serde(default)]
        offset: usize,
    },
    Providers {},
}
pub(crate) async fn dispatch(b: &Bridge, c: Command) -> Result<Value> {
    let app = b.host.application();
    match c {
        Command::Providers {} => app.comparison_providers(),
        Command::Start {
            conversation,
            request,
        } => {
            let mut scope = b.scope(&conversation).await?;
            scope.request_id = request.request_id.clone();
            value(app.start_comparison(&scope, request).await?)
        }
        Command::Status { conversation, job } => value(
            app.comparison_status(&b.scope(&conversation).await?, &job)
                .await?,
        ),
        Command::Cancel { conversation, job } => {
            app.cancel_comparison(&b.scope(&conversation).await?, &job)
                .await?;
            Ok(json!({"cancelled":true}))
        }
        Command::Read { conversation, id } => value(
            app.read_comparison(&b.scope(&conversation).await?, &id)
                .await?,
        ),
        Command::List {
            conversation,
            offset,
        } => {
            let scope = b.scope(&conversation).await?;
            b.bounded_page(offset, |page| {
                let scope = scope.clone();
                async move { value(app.list_comparisons(&scope, page).await?) }
            })
            .await
        }
        Command::Rows {
            conversation,
            id,
            offset,
        } => {
            let scope = b.scope(&conversation).await?;
            b.bounded_page(offset, |page| {
                let (scope, id) = (scope.clone(), id.clone());
                async move { value(app.comparison_rows(&scope, &id, page).await?) }
            })
            .await
        }
        Command::Sources {
            conversation,
            id,
            offset,
        } => {
            let scope = b.scope(&conversation).await?;
            b.bounded_page(offset, |page| {
                let (scope, id) = (scope.clone(), id.clone());
                async move { value(app.comparison_sources(&scope, &id, page).await?) }
            })
            .await
        }
    }
}
