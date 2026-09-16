use lugus_agent::{AgentRuntime, RunOutcome, RunReport, RunRequest, RuntimeEvent, ToolExecutor};
use lugus_app::{ApplicationConfig, Result, conversations::*};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, mpsc, watch};

#[derive(Clone)]
struct Selected {
    current: Arc<AtomicUsize>,
    pinned: Option<usize>,
    observed: Arc<Mutex<Vec<usize>>>,
    interpreting: Arc<Notify>,
    resume: Arc<Notify>,
}
#[async_trait::async_trait]
impl RuntimeFactory for Selected {
    fn snapshot(&self) -> Option<Arc<dyn RuntimeFactory>> {
        Some(Arc::new(Self {
            pinned: Some(self.current.load(Ordering::SeqCst)),
            ..self.clone()
        }))
    }
    fn run_timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(
            5 + self.pinned.unwrap_or(0) as u64,
        ))
    }
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        let selected = self
            .pinned
            .unwrap_or_else(|| self.current.load(Ordering::SeqCst));
        Ok(Box::new(Self {
            pinned: Some(selected),
            ..self.clone()
        }))
    }
}
#[async_trait::async_trait]
impl AgentRuntime for Selected {
    async fn run(
        &mut self,
        request: RunRequest,
        _: &dyn ToolExecutor,
        _: mpsc::Sender<RuntimeEvent>,
        _: watch::Receiver<bool>,
    ) -> lugus_agent::Result<RunReport> {
        let selected = self.pinned.unwrap();
        assert!(request.limits.timeout <= std::time::Duration::from_secs(5 + selected as u64));
        self.observed.lock().unwrap().push(selected);
        let final_text = if request.tools.is_empty() {
            self.interpreting.notify_one();
            self.resume.notified().await;
            r#"{"workflow":"conversation","subjects":[],"start":null,"end":null,"clarification":null}"#.into()
        } else {
            format!("agent {selected}")
        };
        Ok(RunReport {
            run_id: request.run_id,
            outcome: RunOutcome::Completed,
            final_text,
        })
    }
    async fn close(&mut self) -> lugus_agent::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn changing_selection_during_interpretation_only_affects_the_next_message() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("application.json");
    std::fs::write(&config, r#"{"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[]}"#).unwrap();
    let app = ApplicationConfig::load(config)
        .await
        .unwrap()
        .open(false)
        .await
        .unwrap();
    let selected = Selected {
        current: Arc::new(AtomicUsize::new(0)),
        pinned: None,
        observed: Arc::default(),
        interpreting: Arc::default(),
        resume: Arc::default(),
    };
    let host = ConversationHost::start(
        app,
        Arc::new(selected.clone()),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let chat = host.create("create", "Agent selection").await.unwrap();
    let request = |id: &str| SendMessageRequest {
        research_brief: None,
        conversation_id: chat.id.clone(),
        request_id: id.into(),
        text: "Hello".into(),
        selected: vec![],
        company_hint: None,
    };
    let run = host.send(request("first")).await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        selected.interpreting.notified(),
    )
    .await
    .unwrap();
    selected.current.store(1, Ordering::SeqCst);
    selected.resume.notify_one();
    let completed = host.wait(&chat.id, &run.id).await.unwrap();
    assert_eq!(
        completed.status,
        RunStatus::Completed,
        "{:?}",
        completed.error
    );
    assert_eq!(*selected.observed.lock().unwrap(), vec![0, 0]);
    let next = host.send(request("second")).await.unwrap();
    selected.resume.notify_one();
    assert_eq!(
        host.wait(&chat.id, &next.id).await.unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(*selected.observed.lock().unwrap(), vec![0, 0, 1, 1]);
    host.shutdown().await.unwrap();
}
