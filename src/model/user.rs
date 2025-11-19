use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct UserRow {
    pub _id: String,
    pub name: String,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub github_url: Option<String>,
    pub bilibili_url: Option<String>,
}
