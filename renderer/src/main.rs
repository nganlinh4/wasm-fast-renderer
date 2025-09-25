mod types; mod jobs; mod ffmpeg;
use axum::{routing::{get, post}, Json, Router, extract::{Path, State}};
use jobs::{Job, JobStatus, JobStore};
use std::{net::SocketAddr, path::PathBuf, collections::HashMap};
use tokio::{process::Command, io::{AsyncBufReadExt, BufReader}};
use tracing::info;
use types::{DesignEnvelope, StatusResponse, SubmitResponse};

#[derive(Clone)]
struct AppState {
    store: JobStore,
    base_url: String,
    caps: ffmpeg::BackendCaps,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let caps = ffmpeg::detect_caps().await;
    info!(?caps, "Detected backend capabilities");

    let store = JobStore::default();
    let port: u16 = std::env::var("RENDER_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(6108);
    let base_url = format!("http://127.0.0.1:{}", port);

    let state = AppState { store, base_url: base_url.clone(), caps };

    let app = Router::new()
        .route("/render", post(submit_render))
        .route("/render/:id", get(get_status))
        .route("/render/:id/output", get(get_output))
        .with_state(state);

    let addr = SocketAddr::from(([127,0,0,1], port));
    info!(?addr, "Renderer listening");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn submit_render(State(state): State<AppState>, Json(env): Json<DesignEnvelope>) -> Result<Json<SubmitResponse>, (axum::http::StatusCode, String)> {
    let mut design = env.design;
    // Merge optional render options into the design so downstream logic can use a single source
    if let Some(opts) = env.options {
        if let Some(fps) = opts.fps { design.fps = Some(fps); }
        if let Some(sz) = opts.size { design.size = Some(sz); }
    }
    let workdir = std::env::current_dir().unwrap().join("render_jobs");
    let _ = tokio::fs::create_dir_all(&workdir).await;
    let job = Job::new(workdir.join(uuid::Uuid::new_v4().to_string()));
    let job_id = job.id;
    let job_dir = job.workdir.clone();
    state.store.insert(job).await;

    let store = state.store.clone();
    let caps = state.caps.clone();

    // Spawn worker
    tokio::spawn(async move {
        if let Err(e) = tokio::fs::create_dir_all(&job_dir).await { store.update(&job_id, |j| { j.status = JobStatus::Failed; j.error = Some(e.to_string()); }).await; return; }

        // Collect items with src and group by type, download assets
        let mut assets: Vec<(usize, types::TrackItem, PathBuf)> = Vec::new();
        let mut idx = 0usize;
        let items: Vec<types::TrackItem> = if !design.trackItems.is_empty() { design.trackItems.clone() } else { design.trackItemsMap.values().cloned().collect() };
        for it in items.into_iter() {
            let src = it.details.as_ref().and_then(|d| d.src.clone());
            if let Some(url) = src {
                match ffmpeg::download_asset(&url, &job_dir).await {
                    Ok(path) => { assets.push((idx, it, path)); idx += 1; },
                    Err(e) => { store.update(&job_id, |j| { j.status = JobStatus::Failed; j.error = Some(format!("download failed: {}", e)); }).await; return; }
                }
            }
        }
        if assets.is_empty() { store.update(&job_id, |j| { j.status = JobStatus::Failed; j.error = Some("no assets with 'src' found".into()); }).await; return; }

        // Download fonts for text items
        let items2: Vec<types::TrackItem> = if !design.trackItems.is_empty() { design.trackItems.clone() } else { design.trackItemsMap.values().cloned().collect() };
        let mut font_map: HashMap<String, PathBuf> = HashMap::new();
        for it in items2.into_iter() {
            if let types::TrackType::Text = it.kind {
                if let Some(url) = it.details.as_ref().and_then(|d| d.fontUrl.clone()) {
                    match ffmpeg::download_asset(&url, &job_dir).await {
                        Ok(path) => { if let Some(id) = &it.id { font_map.insert(id.clone(), path); } },
                        Err(e) => { store.update(&job_id, |j| { j.status = JobStatus::Failed; j.error = Some(format!("font download failed: {}", e)); }).await; return; }
                    }
                }
            }
        }

        // Build command
        let built = match ffmpeg::build_ffmpeg_command(&job_dir, &design, &assets.iter().map(|(i,it,p)|( *i, it, p.clone())).collect::<Vec<_>>(), &caps, &font_map) {
            Ok(b) => b,
            Err(e) => { store.update(&job_id, |j| { j.status = JobStatus::Failed; j.error = Some(format!("build failed: {}", e)); }).await; return; }
        };

        // Run ffmpeg
        tracing::info!("Launching ffmpeg: ffmpeg {}", built.args.join(" "));

        store.update(&job_id, |j| j.status = JobStatus::Running).await;
        let mut cmd = Command::new("ffmpeg");
        for a in &built.args { cmd.arg(a); }
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = match cmd.spawn() { Ok(c) => c, Err(e) => { store.update(&job_id, |j| { j.status = JobStatus::Failed; j.error = Some(format!("spawn failed: {}", e)); }).await; return; } };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Drain stderr to avoid pipe blocking; also log errors
        let store_err = store.clone();
        let job_err = job_id.clone();
        let mut err_reader = BufReader::new(stderr).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = err_reader.next_line().await {
                tracing::warn!("ffmpeg stderr: {}", line);
                // Optional: detect failure keywords and mark job as failed early if desired
                if line.contains("Error") || line.contains("error:") {
                    // just log; final status handled after wait()
                    let _ = &store_err; let _ = &job_err;
                }
            }
        });

        // Read progress from stdout
        let total_ms = ffmpeg::compute_duration_ms(&design) as f64;
        let mut out_reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = out_reader.next_line().await {
            if let Some(val) = line.strip_prefix("out_time_ms=") {
                if let Ok(mut n) = val.trim().parse::<u64>() {
                    // Some ffmpeg builds output microseconds here; normalize to ms if too large
                    if (n as f64) > total_ms * 1000.0 { n /= 1000; }
                    let pct = ((n as f64 / total_ms) * 100.0).clamp(0.0, 100.0) as u32;
                    store.update(&job_id, |j| j.progress = pct).await;
                }
            }
        }

        let status = child.wait().await;
        match status {
            Ok(s) if s.success() => {
                let out = job_dir.join("output.mp4");
                store.update(&job_id, |j| { j.status = JobStatus::Completed; j.progress = 100; j.output_path = Some(out); }).await;
            }
            Ok(s) => { store.update(&job_id, |j| { j.status = JobStatus::Failed; j.error = Some(format!("ffmpeg exit status: {}", s)); }).await; }
            Err(e) => { store.update(&job_id, |j| { j.status = JobStatus::Failed; j.error = Some(format!("wait failed: {}", e)); }).await; }
        }
    });

    Ok(Json(SubmitResponse { jobId: job_id.to_string() }))
}

async fn get_status(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<StatusResponse>, (axum::http::StatusCode, String)> {
    let uid = uuid::Uuid::parse_str(&id).map_err(|_| (axum::http::StatusCode::BAD_REQUEST, "invalid id".into()))?;
    match state.store.get(&uid).await {
        Some(job) => Ok(Json(job.to_status_response(&state.base_url))),
        None => Err((axum::http::StatusCode::NOT_FOUND, "not found".into())),
    }
}

async fn get_output(State(state): State<AppState>, Path(id): Path<String>) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    let uid = uuid::Uuid::parse_str(&id).map_err(|_| (axum::http::StatusCode::BAD_REQUEST, "invalid id".into()))?;
    if let Some(job) = state.store.get(&uid).await {
        if let Some(path) = job.output_path {
            let bytes = tokio::fs::read(&path).await.map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let mut resp = axum::response::Response::new(bytes.into());
            resp.headers_mut().insert(axum::http::header::CONTENT_TYPE, axum::http::HeaderValue::from_static("video/mp4"));
            Ok(resp)
        } else { Err((axum::http::StatusCode::BAD_REQUEST, "not ready".into())) }
    } else {
        Err((axum::http::StatusCode::NOT_FOUND, "not found".into()))
    }
}

