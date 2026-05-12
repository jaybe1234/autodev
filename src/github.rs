use eyre::WrapErr;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct GitHubClient {
    token: String,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct UserResponse {
    login: String,
}

impl GitHubClient {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn get_authenticated_user(&self) -> eyre::Result<String> {
        let user: UserResponse = self
            .client
            .get("https://api.github.com/user")
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "autodev")
            .send()
            .await
            .with_context(|| "fetching GitHub authenticated user")?
            .json()
            .await
            .with_context(|| "parsing GitHub user response")?;
        Ok(user.login)
    }

    #[allow(dead_code)]
    pub async fn post_pr_comment(
        &self,
        repo: &str,
        pr_number: i64,
        body: &str,
    ) -> eyre::Result<()> {
        let url = format!(
            "https://api.github.com/repos/{}/issues/{}/comments",
            repo, pr_number
        );

        self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "autodev")
            .json(&json!({ "body": body }))
            .send()
            .await
            .with_context(|| format!("posting comment to PR #{pr_number} in {repo}"))?
            .error_for_status()
            .with_context(|| {
                format!("GitHub API error posting comment to PR #{pr_number} in {repo}")
            })?;

        Ok(())
    }
}
