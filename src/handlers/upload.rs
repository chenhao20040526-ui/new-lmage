use crate::core::{cors, crypto, tg};
use crate::domain::image::{self, Image};
use worker::d1::D1Database;
use worker::*;

/// 游客上传限速：每分钟最多 5 张（D1 计数，窗口按分钟滚动）
const GUEST_RATE_LIMIT_PER_MIN: i64 = 5;
const RATE_WINDOW_MS: i64 = 60_000;

async fn enforce_guest_rate_limit(db: &D1Database) -> Result<()> {
    let now = Date::now().as_millis() as i64;

    #[derive(serde::Deserialize)]
    struct LimitRow {
        window_start: i64,
        count: i64,
    }

    let row = db
        .prepare("SELECT window_start, count FROM upload_limits WHERE key = 'anonymous'")
        .first::<LimitRow>(None)
        .await?;

    match row {
        None => {
            db.prepare(
                "INSERT INTO upload_limits (key, window_start, count) VALUES ('anonymous', ?, 1)",
            )
            .bind(&[crate::domain::db::int_i64(now)])?
            .run()
            .await?;
            Ok(())
        }
        Some(r) => {
            if now - r.window_start >= RATE_WINDOW_MS {
                // 窗口过期，重置计数
                db.prepare("UPDATE upload_limits SET window_start = ?, count = 1 WHERE key = 'anonymous'")
                    .bind(&[crate::domain::db::int_i64(now)])?
                    .run()
                    .await?;
                Ok(())
            } else if r.count >= GUEST_RATE_LIMIT_PER_MIN {
                Err(Error::from("游客上传限速：每分钟最多 5 张，登录后不限速"))
            } else {
                db.prepare(
                    "UPDATE upload_limits SET count = count + 1 WHERE key = 'anonymous'",
                )
                .run()
                .await?;
                Ok(())
            }
        }
    }
}

pub async fn upload(mut req: Request, env: Env) -> Result<Response> {
    let db = env.d1("DB")?;
let raw_bot_token = env.var("TG_Bot_Token")?.to_string();
let raw_chat_id = env.var("TG_Chat_ID")?.to_string();

let tokens: Vec<&str> = raw_bot_token.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
let chat_ids: Vec<&str> = raw_chat_id.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

// 生成唯一随机索引，保证 Bot 与 Channel 绑定配对
let rand_index = if !tokens.is_empty() {
    (js_sys::Math::random() * tokens.len() as f64) as usize
} else {
    0
};

let bot_token = tokens.get(rand_index).unwrap_or(&raw_bot_token.as_str()).to_string();
let chat_id = if !chat_ids.is_empty() {
    chat_ids[rand_index % chat_ids.len()].to_string()
} else {
    raw_chat_id
};
    let jwt_secret = env.var("JWT_SECRET")?.to_string();

    // 1. 鉴权判定 (可选)
    let user_id = match crypto::authenticate(&req, &jwt_secret) {
        Ok(claims) => claims.sub,
        Err(_) => "anonymous".to_string(),
    };

    // 1.5 游客限速：未登录每分钟最多 5 张，登录用户不限
    if user_id == "anonymous" {
        if let Err(rate_err) = enforce_guest_rate_limit(&db).await {
            return Response::error(rate_err.to_string(), 429);
        }
    }

    // 2. 文件上传体限制 (20MB)
    if let Some(content_length) = req.headers().get("content-length")? {
        if let Ok(size) = content_length.parse::<usize>() {
            if size > 20 * 1024 * 1024 {
                return Response::error("文件超过 20MB 限制", 413);
            }
        }
    }

    // 3. 读取表单数据
    let form = match req.form_data().await {
        Ok(f) => f,
        Err(_) => return Response::error("表单解析错误", 400),
    };

    let file_entry = match form.get("file") {
        Some(FormEntry::File(file)) => file,
        _ => return Response::error("未检测到文件字段 'file'", 400),
    };

    let filename = file_entry.name();
    let mime_type = file_entry.type_();
    let file_bytes = file_entry.bytes().await?;
    let file_size = file_bytes.len() as i64;

    // 可选归属相册参数
    let album_id = match form.get("albumid") {
        Some(FormEntry::Field(id)) => {
            if id.trim().is_empty() {
                None
            } else {
                Some(id.trim().to_string())
            }
        }
        _ => None,
    };

    // 4. 调用 TG 基建发送文档并解析响应
    let tg_result = match tg::send_document(&bot_token, &chat_id, &filename, &mime_type, &file_bytes).await {
        Ok(res) => res,
        Err(err) => return Response::error(format!("TG 中转异常: {}", err), 502),
    };

    let message_id = tg_result.message_id;
    let doc = match tg_result.document {
        Some(d) => d,
        None => return Response::error("TG 中转元数据提取失败", 502),
    };

    let thumb_file_id = doc.thumbnail.as_ref().map(|t| t.file_id.clone());
    let ext = filename.split('.').last().unwrap_or("png");
    let image_id = format!("{}.{}", doc.file_id, ext);
    let now = Date::now().as_millis() as i64;

    // 5. 写入 D1 数据库
    let new_image = Image {
        id: image_id.clone(),
        file_id: doc.file_id,
        file_name: filename,
        file_size,
        mime_type,
        user_id,
        album_id,
        message_id: Some(message_id),
        thumb_file_id,
        views: 0,
        last_accessed_at: None,
        is_trash: 0,
        is_blocked: 0,
        uploaded_at: now,
    };

    image::create(&db, &new_image).await?;

    let output = serde_json::json!([{
        "src": format!("/file/{}", image_id)
    }]);

    let headers = cors::apply_cors(Headers::new())?;
    headers.set("Content-Type", "application/json")?;

    let response = Response::from_json(&output)?.with_headers(headers);
    Ok(response)
}
