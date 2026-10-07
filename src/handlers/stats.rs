use crate::core::{cors, crypto};
use worker::*;

// 获取用户资产统计
pub async fn get_user_stats(req: Request, env: Env) -> Result<Response> {
    let db = env.d1("DB")?;
    let jwt_secret = env.var("JWT_SECRET")?.to_string();
    let claims = match crypto::authenticate(&req, &jwt_secret) {
        Ok(c) => c,
        Err(err) => return Response::error(err.to_string(), 401),
    };

    // 1) 统计图片总数与总存储占用 (is_trash = 0)
    let img_stats = db.prepare(
        "SELECT COUNT(*) as total_count, COALESCE(SUM(file_size), 0) as total_size 
         FROM images WHERE user_id = ? AND is_trash = 0"
    )
    .bind(&[claims.sub.clone().into()])?
    .first::<serde_json::Value>(None)
    .await?;

    let total_count = img_stats.as_ref()
        .and_then(|v| v.get("total_count"))
        .and_then(|c| c.as_i64())
        .unwrap_or(0);

    let total_size = img_stats.as_ref()
        .and_then(|v| v.get("total_size"))
        .and_then(|s| s.as_i64())
        .unwrap_or(0);

    // 2) 统计相册总数
    let album_stats = db.prepare("SELECT COUNT(*) as count FROM albums WHERE user_id = ?")
        .bind(&[claims.sub.clone().into()])?
        .first::<serde_json::Value>(None)
        .await?;
    
    let total_albums = album_stats.as_ref()
        .and_then(|v| v.get("count"))
        .and_then(|c| c.as_i64())
        .unwrap_or(0);

    // 3) 统计标签总数
    let tag_stats = db.prepare("SELECT COUNT(*) as count FROM tags WHERE user_id = ?")
        .bind(&[claims.sub.into()])?
        .first::<serde_json::Value>(None)
        .await?;
    
    let total_tags = tag_stats.as_ref()
        .and_then(|v| v.get("count"))
        .and_then(|c| c.as_i64())
        .unwrap_or(0);

    let headers = cors::apply_cors(Headers::new())?;
    headers.set("Content-Type", "application/json")?;

    let quota_limit_bytes = 1024.0 * 1024.0 * 1024.0 * 1024.0;

    let response = Response::from_json(&serde_json::json!({
        "stats": {
            "totalImages": total_count,
            "totalSize": total_size,
            "totalAlbums": total_albums,
            "totalTags": total_tags,
            "quotaLimit": quota_limit_bytes as i64, 
            "quotaUsedPercentage": if total_size > 0 {
                ((total_size as f64 / quota_limit_bytes) * 100.0).min(100.0)
            } else {
                0.0
            }
        }
    }))?.with_headers(headers);

    Ok(response)
}
