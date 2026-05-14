use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct JiraWebhookPayload {
    #[serde(rename = "webhookEvent")]
    pub webhook_event: Option<String>,
    #[serde(rename = "issue_event_type_name")]
    pub event_type: Option<String>,
    pub issue: Option<JiraIssue>,
    pub changelog: Option<JiraChangelog>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JiraIssue {
    pub key: String,
    pub fields: JiraIssueFields,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JiraIssueFields {
    pub summary: String,
    pub description: Option<String>,
    pub labels: Vec<String>,
    pub status: JiraStatus,
    #[serde(rename = "issuetype")]
    pub issue_type: JiraIssueType,
    pub project: JiraProject,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JiraStatus {
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JiraIssueType {
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JiraProject {
    pub key: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JiraChangelog {
    pub items: Vec<JiraChangelogItem>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JiraChangelogItem {
    pub field: String,
    #[serde(rename = "toString")]
    pub to_string: String,
    #[serde(rename = "fromString")]
    pub from_string: Option<String>,
}
