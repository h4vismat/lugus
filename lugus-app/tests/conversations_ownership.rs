use lugus_app::{ErrorKind, conversations::LocalExecutionLease};
#[test]
fn competing_handles_aliases_and_drop_reacquire() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    std::fs::write(&path, []).unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    assert_eq!(
        LocalExecutionLease::acquire(&path).unwrap_err().kind,
        ErrorKind::Conflict
    );
    #[cfg(unix)]
    {
        let alias = dir.path().join("alias.sqlite");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert_eq!(
            LocalExecutionLease::acquire(alias).unwrap_err().kind,
            ErrorKind::Conflict
        );
    }
    drop(lease);
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    drop(lease);
    assert!(dir.path().join("app.sqlite.conversation.lock").exists());
}
#[test]
fn process_cannot_acquire_a_live_parents_lease() {
    const CHILD: &str = "LUGUS_LEASE_TEST_CHILD_DB";
    if let Some(path) = std::env::var_os(CHILD) {
        assert_eq!(
            LocalExecutionLease::acquire(path).unwrap_err().kind,
            ErrorKind::Conflict
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    std::fs::write(&path, []).unwrap();
    let _lease = LocalExecutionLease::acquire(&path).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process_cannot_acquire_a_live_parents_lease",
            "--nocapture",
        ])
        .env(CHILD, &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
