use serde::{Deserialize, Serialize};

// #[derive(Debug, Deserialize, Serialize)]
// pub struct PostRow {
//     pub _id: i32,
//     pub title: String,
//     pub excerpt: String,
//     #[serde(with = "time::serde::rfc3339")]
//     pub date: time::OffsetDateTime,
//     pub category: String,
//     pub content: String,
//     pub tags: Option<Vec<String>>,
//     pub cover_image: String,
//     pub word_count: i32,
//     pub read_time: i32,
// }
#[derive(Debug, Deserialize, Serialize)]
pub struct PostDetailDTO {
    pub _id: i32,
    pub title: String,
    pub excerpt: String,
    #[serde(with = "time::serde::rfc3339")]
    pub date: time::OffsetDateTime,
    pub category: String,
    pub content: String,
    pub cover_image: String,
    // R2 keys(D1 新增列,老数据为 NULL/"";serde default 兼容未迁移的库)
    #[serde(default)]
    pub content_key: Option<String>,
    #[serde(default)]
    pub cover_key: Option<String>,
    pub tags: Option<Vec<String>>,
    pub word_count: i32,
    pub read_time: i32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PostInsertDTO {
    pub title: String,
    pub excerpt: String,
    #[serde(with = "time::serde::rfc3339")]
    pub date: time::OffsetDateTime,
    pub category: String,
    // 新流程:正文以 content_key 为主;content 为空或兼容老客户端直传全文
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub cover_image: String,
    #[serde(default)]
    pub content_key: Option<String>,
    #[serde(default)]
    pub cover_key: Option<String>,
    pub tags: Option<Vec<String>>,
    pub word_count: i32,
    pub read_time: i32,
}
