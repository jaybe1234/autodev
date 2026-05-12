use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::db::Db;
use tokio::sync::mpsc::UnboundedSender;

pub struct QueuedReview {
    pub pr_repo: String,
    pub pr_number: i64,
    pub branch_name: String,
    pub reviewer_login: String,
    pub review_body: String,
    pub review_state: String,
    pub source: String,
    pub parent_task_id: String,
    #[allow(dead_code)]
    pub session_id: String,
    pub repo_url: String,
}

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub db: Db,
    pub github_username: String,
    pub review_queue: Arc<Mutex<HashMap<String, VecDeque<QueuedReview>>>>,
    pub review_notify: UnboundedSender<String>,
}
