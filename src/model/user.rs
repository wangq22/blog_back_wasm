use serde::{Deserialize, Serialize};

/// user 表整行(单用户博客,全站只有一行)。
/// 新增 timezone/city/email/affiliation 后 GET /api/user(SELECT *) 自动带回,无需改查询。
#[derive(Debug, Deserialize, Serialize)]
pub struct UserRow {
    pub _id: i32,
    pub name: String,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub github_url: Option<String>,
    #[serde(default)]
    pub bilibili_url: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub affiliation: Option<String>,
}

/// 更新用户资料:全字段可选,只传要改的即可,缺省字段保持原值。
/// 空字符串表示清空该字段。
#[derive(Debug, Deserialize, Serialize)]
pub struct UserUpdateDTO {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub github_url: Option<String>,
    #[serde(default)]
    pub bilibili_url: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub affiliation: Option<String>,
}
