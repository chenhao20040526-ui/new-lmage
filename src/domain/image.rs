use serde::{Deserialize, Serialize};
use worker::d1::D1Database;
use worker::*;

use super::db::{int_i64, opt_i64, opt_str};

// 图片实体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub id: String,
    pub file_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub mime_type: String,
    pub user_id: String,
    pub album_id: Option<String>,
    pub message_id: Option<i64>,
    pub chat_id: Option<String>, // 👈 补上这个缺失的字段，报错 1/2 解决！
    pub thumb_file_id: Option<String>,
    pub views: i64,
    pub last_accessed_at: Option<i64>,
    pub is_trash: i32,
    pub is_blocked: i32,
    pub uploaded_at: i64,
}

// 批量打标载荷
#[derive(Debug, Deserialize)]
pub struct BatchImageTagRequest {
    #[serde(alias = "file_ids")]
    pub file_ids: Vec<String>,
    pub tags: Vec<String>,
    pub action: String,
}

// ==================== D1 CRUD 操作 ====================

pub async fn create(db: &D1Database, img: &Image) -> Result<()> {
    db.prepare(
        // 👇 检查：这里一共声明了 15 个字段，VALUES 后面是 15 个问号 ?
        "INSERT INTO images (
            id, file_id, file_name, file_size, mime_type, 
            user_id, album_id, message_id, chat_id, thumb_file_id, 
            views, last_accessed_at, is_trash, is_blocked, uploaded_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&[
        img.id.as_str().into(),                 // 1. id
        img.file_id.as_str().into(),           // 2. file_id
        img.file_name.as_str().into(),         // 3. file_name
        int_i64(img.file_size),                // 4. file_size
        img.mime_type.as_str().into(),         // 5. mime_type
        img.user_id.as_str().into(),           // 6. user_id
        opt_str(img.album_id.as_deref()),      // 7. album_id
        opt_i64(img.message_id),               // 8. message_id
        opt_str(img.chat_id.as_deref()),       // 9. chat_id (👈 核心新增)
        opt_str(img.thumb_file_id.as_deref()), // 10. thumb_file_id
        int_i64(img.views),                    // 11. views
        opt_i64(img.last_accessed_at),         // 12. last_accessed_at
        img.is_trash.into(),                   // 13. is_trash
        img.is_blocked.into(),                 // 14. is_blocked
        int_i64(img.uploaded_at),              // 15. uploaded_at
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn get_by_id(db: &D1Database, id: &str) -> Result<Option<Image>> {
    db.prepare("SELECT * FROM images WHERE id = ?")
        .bind(&[id.into()])?
        .first::<Image>(None)
        .await
}

pub async fn delete_physically(db: &D1Database, id: &str) -> Result<()> {
    // 先清关联标签（双保险：不依赖外键级联是否生效）
    db.prepare("DELETE FROM image_tags WHERE image_id = ?")
        .bind(&[id.into()])?
        .run()
        .await?;
    db.prepare("DELETE FROM images WHERE id = ?")
        .bind(&[id.into()])?
        .run()
        .await?;
    Ok(())
}

pub async fn increment_views(db: &D1Database, id: &str, now: i64) -> Result<()> {
    db.prepare("UPDATE images SET views = views + 1, last_accessed_at = ? WHERE id = ?")
        .bind(&[int_i64(now), id.into()])?
        .run()
        .await?;
    Ok(())
}
