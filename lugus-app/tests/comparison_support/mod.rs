#![allow(dead_code)]
use lugus_app::{comparison::*, *};
pub fn request(id: &str) -> ComparisonRequest {
    serde_json::from_value(serde_json::json!({"request_id":id,"subjects":[{"text":"AAPL","exchange":null},{"text":"MSFT","exchange":null}],"period_end":"2024-12-31","years":3,"revenue_basis":"contract_revenue_excluding_tax"})).unwrap()
}
pub async fn terminal(app: &Application, scope: &Scope, id: &str) -> ComparisonJob {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let j = app.comparison_status(scope, id).await.unwrap();
            if j.state.terminal() {
                return j;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
pub async fn offline(fin: &std::path::Path, db: &std::path::Path) -> Application {
    let repos = std::sync::Arc::new(SqliteRepositoryFactory::new(fin));
    let s = SqliteApplicationStore::open(
        db,
        Box::new(lugus_financial::storage::SqliteRepository::open(fin).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    Application::start(
        vec![],
        repos,
        Box::new(s),
        Limits::default(),
        HostBounds::default(),
        Box::new(RandomIds::new().unwrap()),
    )
    .await
    .unwrap()
}
