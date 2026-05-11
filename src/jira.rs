use eyre::WrapErr;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct JiraClient {
    base_url: String,
    pat: String,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct TransitionResponse {
    transitions: Vec<Transition>,
}

#[derive(Debug, Deserialize)]
struct Transition {
    id: String,
    name: String,
}

impl JiraClient {
    pub fn new(base_url: &str, pat: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            pat: pat.to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn add_comment(&self, issue_key: &str, body: &str) -> eyre::Result<()> {
        let url = format!(
            "{}/rest/api/2/issue/{}/comment",
            self.base_url, issue_key
        );

        self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.pat))
            .header("Content-Type", "application/json")
            .json(&json!({ "body": body }))
            .send()
            .await
            .with_context(|| format!("adding comment to {issue_key}"))?
            .error_for_status()
            .with_context(|| format!("Jira API error adding comment to {issue_key}"))?;

        Ok(())
    }

    pub async fn transition_issue(
        &self,
        issue_key: &str,
        target_status: &str,
    ) -> eyre::Result<()> {
        let transitions_url = format!(
            "{}/rest/api/2/issue/{}/transitions",
            self.base_url, issue_key
        );

        let transitions: TransitionResponse = self
            .client
            .get(&transitions_url)
            .header("Authorization", format!("Bearer {}", self.pat))
            .send()
            .await
            .with_context(|| format!("fetching transitions for {issue_key}"))?
            .json()
            .await
            .with_context(|| format!("parsing transitions for {issue_key}"))?;

        let transition = transitions
            .transitions
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(target_status))
            .ok_or_else(|| {
                eyre::eyre!(
                    "transition '{target_status}' not found for {issue_key}. Available: {:?}",
                    transitions
                        .transitions
                        .iter()
                        .map(|t| &t.name)
                        .collect::<Vec<_>>()
                )
            })?;

        self.client
            .post(&transitions_url)
            .header("Authorization", format!("Bearer {}", self.pat))
            .header("Content-Type", "application/json")
            .json(&json!({ "transition": { "id": transition.id } }))
            .send()
            .await
            .with_context(|| format!("transitioning {issue_key} to {target_status}"))?
            .error_for_status()
            .with_context(|| format!("Jira API error transitioning {issue_key}"))?;

        Ok(())
    }
}
