use desktop_host::Bridge;
use lugus_app::{AppError, ErrorKind};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::Manager;
struct Desktop {
    bridge: Option<Bridge>,
    startup_error: Option<AppError>,
}
#[tauri::command]
async fn research(
    window: tauri::WebviewWindow,
    desktop: tauri::State<'_, Desktop>,
    payload: String,
) -> Result<serde_json::Value, AppError> {
    if window.label() != "main" {
        return Err(AppError::new(
            ErrorKind::ScopeMismatch,
            "untrusted desktop window",
            false,
        ));
    }
    match &desktop.bridge {
        Some(bridge) => bridge.dispatch(&payload).await,
        None => {
            if payload.len() < 256
                && serde_json::from_str::<serde_json::Value>(&payload).ok()
                    == Some(serde_json::json!({"op":"info"}))
            {
                Ok(
                    serde_json::json!({"offline":true,"runtime_available":false,"startup_error":desktop.startup_error.as_ref().map(|e|e.message.clone())}),
                )
            } else {
                Err(desktop.startup_error.clone().unwrap_or_else(|| {
                    AppError::new(ErrorKind::Unavailable, "desktop host is unavailable", false)
                }))
            }
        }
    }
}
fn configuration(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os("LUGUS_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    let directory = app.path().app_data_dir()?;
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("desktop.json");
    if !path.exists() {
        std::fs::create_dir_all(directory.join("runtime"))?;
        let application = directory.join("application.json");
        if !application.exists() {
            std::fs::write(
                &application,
                serde_json::to_vec_pretty(
                    &serde_json::json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[]}),
                )?,
            )?;
        }
        let executable = std::env::var_os("LUGUS_CODEX")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/bin/codex"))
            })
            .filter(|p| p.is_file());
        let runtime=executable.map(|p|serde_json::json!({"executable":p,"workspace":"runtime","model":null,"model_provider":null}));
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(
                &serde_json::json!({"application_config":"application.json","runtime":runtime}),
            )?,
        )?;
    }
    Ok(path)
}
fn main() {
    let stopped = Arc::new(AtomicBool::new(false));
    let stopping = Arc::new(AtomicBool::new(false));
    let app=tauri::Builder::default().invoke_handler(tauri::generate_handler![research]).setup(|app|{
        let host=match configuration(app){Ok(path)=>{let offline=std::env::var_os("LUGUS_OFFLINE").is_some_and(|v|v=="1");tauri::async_runtime::block_on(Bridge::open(&path,offline))},Err(_)=>Err(AppError::new(ErrorKind::Unavailable,"Could not open desktop configuration. Set LUGUS_CONFIG to your desktop.json and restart.",false))};
        app.manage(match host{Ok(bridge)=>Desktop{bridge:Some(bridge),startup_error:None},Err(error)=>Desktop{bridge:None,startup_error:Some(error)}});
        let window=tauri::WebviewWindowBuilder::new(app,"main",tauri::WebviewUrl::App("index.html".into())).title("Lugus").inner_size(1380.0,900.0).min_inner_size(1080.0,650.0).on_navigation(|url|url.scheme()=="tauri"&&url.host_str()==Some("localhost")).build();
        if let Err(error)=window{if let Some(bridge)=&app.state::<Desktop>().bridge{let _=tauri::async_runtime::block_on(bridge.shutdown());}return Err(error.into());}Ok(())
    }).on_window_event(|window,event|{if let tauri::WindowEvent::CloseRequested{api,..}=event{api.prevent_close();window.app_handle().exit(0);}}).build(tauri::generate_context!()).expect("could not start Lugus desktop");
    app.run(move |app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && !stopped.load(Ordering::Acquire)
        {
            api.prevent_exit();
            if !stopping.swap(true, Ordering::AcqRel) {
                let handle = app.clone();
                let bridge = app.state::<Desktop>().bridge.clone();
                let stopped = stopped.clone();
                tauri::async_runtime::spawn(async move {
                    let code = if let Some(bridge) = bridge {
                        if let Err(error) = bridge.shutdown().await {
                            eprintln!("{error}");
                            1
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    stopped.store(true, Ordering::Release);
                    handle.exit(code);
                });
            }
        }
    });
}
