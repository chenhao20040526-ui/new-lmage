use crate::core::{cors, crypto, tg};
use crate::domain::image::{self, Image};
use worker::*;

// 1. 获取图片分页列表（可选鉴权：登录用户看自己的；游客看全站公开画廊）
pub async fn get_user_images(req: Request, env: Env) -> Result<Response> {
    let db = env.d1("DB")?;
    let jwt_secret = env.var("JWT_SECRET")?.to_string();
    let claims = crypto::authenticate(&req, &jwt_secret).ok();

    let url = req.url()?;
    let query = url.query_pairs();
    let mut page = 1;
    let mut limit = 20;

    for (k, v) in query {
        if k == "page" {
            page = v.parse::<i32>().unwrap_or(1);
        } else if k == "limit" {
            limit = v.parse::<i32>().unwrap_or(20);
        }
    }

    let offset = (page - 1) * limit;

    // 登录用户：本人图片；游客：全站公开图片（封禁/回收站除外）
    let (images, total) = match &claims {
        Some(c) => {
            let images = db
                .prepare(
                    "SELECT * FROM images WHERE user_id = ? AND is_trash = 0 AND is_blocked = 0 ORDER BY uploaded_at DESC LIMIT ? OFFSET ?"
                )
                .bind(&[c.sub.clone().into(), limit.into(), offset.into()])?
                .all()
                .await?
                .results::<Image>()?;
            let count_res = db
                .prepare("SELECT COUNT(*) as count FROM images WHERE user_id = ? AND is_trash = 0 AND is_blocked = 0")
                .bind(&[c.sub.clone().into()])?
                .first::<serde_json::Value>(None)
                .await?;
            let total = count_res
                .and_then(|val| val.get("count").and_then(|v| v.as_i64()))
                .unwrap_or(0);
            (images, total)
        }
        None => {
            let images = db
                .prepare(
                    "SELECT * FROM images WHERE is_trash = 0 AND is_blocked = 0 ORDER BY uploaded_at DESC LIMIT ? OFFSET ?"
                )
                .bind(&[limit.into(), offset.into()])?
                .all()
                .await?
                .results::<Image>()?;
            let count_res = db
                .prepare("SELECT COUNT(*) as count FROM images WHERE is_trash = 0 AND is_blocked = 0")
                .first::<serde_json::Value>(None)
                .await?;
            let total = count_res
                .and_then(|val| val.get("count").and_then(|v| v.as_i64()))
                .unwrap_or(0);
            (images, total)
        }
    };

    let headers = cors::apply_cors(Headers::new())?;
    headers.set("Content-Type", "application/json")?;

    let response = Response::from_json(&serde_json::json!({
        "images": images,
        "total": total,
        "page": page,
        "limit": limit
    }))?.with_headers(headers);

    Ok(response)
}

// 2. 获取单张图片元数据详情
pub async fn get_image_detail(req: Request, env: Env, imageid: String) -> Result<Response> {
    let db = env.d1("DB")?;
    let jwt_secret = env.var("JWT_SECRET")?.to_string();
    let claims = match crypto::authenticate(&req, &jwt_secret) {
        Ok(c) => c,
        Err(err) => return Response::error(err.to_string(), 401),
    };

    // D1 存完整 id（纯 file_id + 后缀），直接用完整 imageid 查询
    let img = match image::get_by_id(&db, &imageid).await? {
        Some(i) => i,
        None => return Response::error("图片不存在", 404),
    };

    if img.user_id != claims.sub && claims.username != "admin" {
        return Response::error("无权查看此图片元数据", 403);
    }

    let headers = cors::apply_cors(Headers::new())?;
    headers.set("Content-Type", "application/json")?;

    let response = Response::from_json(&img)?.with_headers(headers);
    Ok(response)
}

// 3. 搜索图片（可选鉴权：登录用户搜自己的；游客搜全站公开画廊）
pub async fn search_user_images(req: Request, env: Env) -> Result<Response> {
    let db = env.d1("DB")?;
    let jwt_secret = env.var("JWT_SECRET")?.to_string();
    let claims = crypto::authenticate(&req, &jwt_secret).ok();

    let url = req.url()?;
    let query = url.query_pairs();
    let mut keyword = String::new();

    for (k, v) in query {
        if k == "keyword" || k == "q" {
            keyword = v.into_owned();
        }
    }

    let like_query = format!("%{}%", keyword);

    let images = match &claims {
        Some(c) => db
            .prepare(
                "SELECT * FROM images WHERE user_id = ? AND is_trash = 0 AND is_blocked = 0 AND file_name LIKE ? ORDER BY uploaded_at DESC"
            )
            .bind(&[c.sub.clone().into(), like_query.into()])?
            .all()
            .await?
            .results::<Image>()?,
        None => db
            .prepare(
                "SELECT * FROM images WHERE is_trash = 0 AND is_blocked = 0 AND file_name LIKE ? ORDER BY uploaded_at DESC"
            )
            .bind(&[like_query.into()])?
            .all()
            .await?
            .results::<Image>()?,
    };

    let headers = cors::apply_cors(Headers::new())?;
    headers.set("Content-Type", "application/json")?;

    let response = Response::from_json(&serde_json::json!({
        "images": images
    }))?.with_headers(headers);

    Ok(response)
}

// 4. 批量物理永久删除（纯图床模式：删除即永久，无回收站概念）
pub async fn batch_permanent_delete(mut req: Request, env: Env) -> Result<Response> {
let db = env.d1("DB")?;
    let raw_bot_token = env.var("TG_Bot_Token")?.to_string();
    let bot_token = raw_bot_token.split(',').next().unwrap_or(&raw_bot_token).trim().to_string();
    let jwt_secret = env.var("JWT_SECRET")?.to_string();
    let claims = match crypto::authenticate(&req, &jwt_secret) {
        Ok(c) => c,
        Err(err) => return Response::error(err.to_string(), 401),
    };

    #[derive(serde::Deserialize)]
    struct BatchRequest {
        #[serde(alias = "file_ids")]
        fileids: Vec<String>,
    }

    let body: BatchRequest = match req.json().await {
        Ok(b) => b,
        Err(_) => return Response::error("请求 JSON 格式错误", 400),
    };

    // 边缘缓存清理需要请求源（origin 前缀拼缓存 key）
    let origin = req
        .url()
        .ok()
        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")));

    let mut success_count = 0;
    for full_id in body.fileids {
        // D1 存完整 id（纯 file_id + 后缀），查询与写入都用完整 id
        if let Ok(Some(img)) = image::get_by_id(&db, &full_id).await {
            if img.user_id == claims.sub || claims.username == "admin" {
                // 1) 销毁 TG 频道原始消息（优先读取数据库存的亲生频道）
                if let Some(msg_id) = img.message_id {
                    let target_chat_id = img.chat_id.as_deref().unwrap_or_else(|| {
                        env.var("TG_Chat_ID").ok().as_deref().and_then(|s| s.split(',').next()).unwrap_or("")
                    });
                    if !target_chat_id.is_empty() {
                        let _ = tg::delete_message(&bot_token, target_chat_id, msg_id).await;
                    }
                }
                // 2) 清边缘缓存（原图 + 缩略图），防止已删图片被缓存继续吐给用户
                if let Some(ref origin) = origin {
                    for suffix in ["", "?size=thumb"] {
                        let cache_key = format!("{}/file/{}{}", origin, full_id, suffix);
                        let _ = Cache::default().delete(&cache_key, true).await;
                    }
                }
                // 3) 抹除 D1 记录（含 image_tags 关联清理）
                let _ = image::delete_physically(&db, &full_id).await;
                // 4) 失效内存状态缓存
                crate::core::img_cache::invalidate(&full_id);
                success_count += 1;
            }
        }
    }

    let headers = cors::apply_cors(Headers::new())?;
    headers.set("Content-Type", "application/json")?;

    let response = Response::from_json(&serde_json::json!({
        "success": true,
        "message": format!("已永久删除 {} 张图片（TG 消息与边缘缓存同步销毁）", success_count)
    }))?.with_headers(headers);

    Ok(response)
}
