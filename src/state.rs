use crate::config::AppConfig;
use crate::db::Db;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub db: Db,
}
