use serde::{Deserialize, Serialize};

// D1 只存 key,正文/封面二进制全在 R2(不兼容老数据的 content/cover_image 列)。
// GET /post/{id} 返回的 `content` 是后端从 R2 读回填的正文(非存储字段)。

/// 详情:含 R2 回填的 `content`。
#[derive(Debug, Deserialize, Serialize)]
pub struct PostDetailDTO {
    pub _id: i32,
    pub title: String,
    pub excerpt: String,
    #[serde(with = "time::serde::rfc3339")]
    pub date: time::OffsetDateTime,
    pub category: String,
    pub content: String,
    pub content_key: String,
    #[serde(default)]
    pub cover_key: Option<String>,
    pub tags: Option<Vec<String>>,
    pub word_count: i32,
    pub read_time: i32,
}

/// 新建:正文必须已先经 POST /api/protected/media 推到 R2,直接给 key。
#[derive(Debug, Deserialize, Serialize)]
pub struct PostInsertDTO {
    pub title: String,
    pub excerpt: String,
    #[serde(with = "time::serde::rfc3339")]
    pub date: time::OffsetDateTime,
    pub category: String,
    pub content_key: String,
    #[serde(default)]
    pub cover_key: Option<String>,
    pub tags: Option<Vec<String>>,
    pub word_count: i32,
    pub read_time: i32,
}

/// 更新:同新建 + _id。
#[derive(Debug, Deserialize, Serialize)]
pub struct PostUpdateDTO {
    pub _id: i32,
    pub title: String,
    pub excerpt: String,
    #[serde(with = "time::serde::rfc3339")]
    pub date: time::OffsetDateTime,
    pub category: String,
    pub content_key: String,
    #[serde(default)]
    pub cover_key: Option<String>,
    pub tags: Option<Vec<String>>,
    pub word_count: i32,
    pub read_time: i32,
}
