use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct PullRequestReviewEvent {
    pub action: String,
    pub review: GitHubReview,
    pub pull_request: GitHubPullRequest,
    pub repository: GitHubRepository,
}

#[derive(Debug, Deserialize)]
pub struct IssueCommentEvent {
    pub action: String,
    pub comment: GitHubComment,
    pub issue: GitHubIssue,
    pub repository: GitHubRepository,
}

#[derive(Debug, Deserialize)]
pub struct GitHubReview {
    #[allow(dead_code)]
    pub id: i64,
    pub body: Option<String>,
    pub state: String,
    pub user: GitHubUser,
}

#[derive(Debug, Deserialize)]
pub struct GitHubPullRequest {
    pub number: i64,
    pub head: GitHubPullRequestHead,
    pub user: GitHubUser,
}

#[derive(Debug, Deserialize)]
pub struct GitHubPullRequestHead {
    #[serde(rename = "ref")]
    pub ref_name: String,
}

#[derive(Debug, Deserialize)]
pub struct GitHubComment {
    #[allow(dead_code)]
    pub id: i64,
    pub body: String,
    pub user: GitHubUser,
}

#[derive(Debug, Deserialize)]
pub struct GitHubIssue {
    pub number: i64,
    pub pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct GitHubRepository {
    pub full_name: String,
    #[allow(dead_code)]
    pub clone_url: String,
}

#[derive(Debug, Deserialize)]
pub struct GitHubUser {
    pub login: String,
}
