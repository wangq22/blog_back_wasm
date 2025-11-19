use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct PostRow {
    pub _id: i32,
    pub title: String,
    pub excerpt: String,
    #[serde(with = "time::serde::rfc3339")]
    pub date: time::OffsetDateTime,
    pub category: String,
    pub content: String,
    pub tags: Option<Vec<String>>,
    pub cover_image: String,
    pub word_count: i32,
    pub read_time: i32,
}

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
    pub content: String,
    pub cover_image: String,
    pub tags: Option<Vec<String>>,
    pub word_count: i32,
    pub read_time: i32,
}
