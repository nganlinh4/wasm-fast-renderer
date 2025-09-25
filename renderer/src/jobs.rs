use crate::types::StatusResponse;
use std::{collections::HashMap, path::PathBuf, time::Instant, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum JobStatus { Pending, Running, Completed, Failed }

#[derive(Debug, Clone)]
pub struct Job {
    pub id: Uuid,
    pub status: JobStatus,
    pub progress: u32,
    pub output_path: Option<PathBuf>,
    pub error: Option<String>,
    pub created_at: Instant,
    pub workdir: PathBuf,
}

impl Job {
    pub fn new(workdir: PathBuf) -> Self {
        Self {
            id: Uuid::new_v4(),
            status: JobStatus::Pending,
            progress: 0,
            output_path: None,
            error: None,
            created_at: Instant::now(),
            workdir,
        }
    }

    pub fn to_status_response(&self, base_url: &str) -> StatusResponse {
        StatusResponse {
            status: match self.status {
                JobStatus::Pending => "PENDING".into(),
                JobStatus::Running => "RUNNING".into(),
                JobStatus::Completed => "COMPLETED".into(),
                JobStatus::Failed => "FAILED".into(),
            },
            progress: self.progress,
            url: self
                .output_path
                .as_ref()
                .map(|_| format!("{}/render/{}/output", base_url, self.id)),
            error: self.error.clone(),
        }
    }
}

#[derive(Clone)]
pub struct JobStore(pub Arc<RwLock<HashMap<Uuid, Job>>>);

impl Default for JobStore {
    fn default() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }
}

impl JobStore {
    pub async fn insert(&self, job: Job) -> Uuid {
        let id = job.id;
        self.0.write().await.insert(id, job);
        id
    }
    pub async fn get(&self, id: &Uuid) -> Option<Job> { self.0.read().await.get(id).cloned() }
    pub async fn update<F: FnOnce(&mut Job)>(&self, id: &Uuid, f: F) {
        if let Some(job) = self.0.write().await.get_mut(id) { f(job); }
    }
}

