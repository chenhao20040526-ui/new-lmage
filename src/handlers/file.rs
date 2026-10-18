use crate::core::{cors, crypto, tg};
use crate::domain::image;
use worker::*;

// 1. 图片代理网关（原图 302 直连 Telegram CDN，缩略图走 Cloudflare Cache API 物理缓存）
pub async fn proxy_file(req: Request, env: Env, filekey: String) -> Result<Response> {
    let db = env.d1("DB")?;
    let raw_bot_token = env.var("TG_Bot_Token")?.to_string();
    let bot_token = raw_bot_token.split(',').next().unwrap_or(&raw_bot_token).trim().to_string();

    // ⚡ 状态管理开关（默认关闭 = 纯图床模式）：
    // - off：上传后完全不管，图片访问路径零状态校验、零点击统计写入，
    //   仅按需读取缩略图元数据（30s 内存缓存 + 365 天边缘缓存吸收，实际趋近零查询）
    // - on：执行封禁/回收站校验（校验必须先于缓存检查，防绕过）与点击统计
    let status_check_on = env
        .var("FILE_STATUS_CHECK")
        .map(|v| v.to_string().eq_ignore_ascii_case("on"))
        .unwrap_or(false);

    let raw_file_id = filekey.split('.').next().unwrap_or(&filekey).to_string();
    let mut target_file_id = raw_file_id.clone();

    // 缩略图元数据：内存缓存优先（30s），未命中查 D1 回填
    let img_opt = match crate::core::img_cache::get(&filekey) {
        Some(img) => Some(img),
        None => {
            let opt = image::get_by_id(&db, &filekey).await.unwrap_or(None);
            if let Some(ref img) = opt {
                crate::core::img_cache::set(&filekey, img);
            }
            opt
        }
    };

    if status_check_on {
        // 状态校验模式：封禁/回收站必须立即生效
        if let Some(ref img) = img_opt {
            if img.is_blocked == 1 {
                let headers = cors::apply_cors(Headers::new())?;
                headers.set("Content-Type", "application/json")?;
                return Ok(Response::error("图片已被封禁", 403)?.with_headers(headers));
            }
            if img.is_trash == 1 {
                let headers = cors::apply_cors(Headers::new())?;
                headers.set("Content-Type", "application/json")?;
                return Ok(Response::error("图片已被移入回收站", 404)?.with_headers(headers));
            }
        }
    }

    // size=thumb 缩略图快速白嫖：完美保留！
    let url = req.url()?;
    let mut request_thumb = false;
    for (k, v) in url.query_pairs() {
        if k == "size" && v == "thumb" {
            request_thumb = true;
            break;
        }
    }

    if request_thumb {
        if let Some(ref img) = img_opt {
            if let Some(ref thumb_id) = img.thumb_file_id {
                target_file_id = thumb_id.clone();
            }
        }
    }

    // 1) 向 TG 换取物理路径
    let file_path = match tg::get_file_path(&bot_token, &target_file_id).await {
        Ok(path) => path,
        Err(err) => return Response::error(err.to_string(), 404),
    };

    // 2) 组装 TG 官方 CDN 真实直链地址
    let tg_file_url = format!("https://api.telegram.org/file/bot{}/{}", bot_token, file_path);

    // 🚀 核心改动：如果是原图请求（非 thumb），直接返回 302 重定向给客户端，彻底消除中转断流与卡死！
    if !request_thumb {
        // 累计 Views (仅状态管理开启时统计)
        if status_check_on && img_opt.is_some() {
            let now = Date::now().as_millis() as i64;
            let _ = image::increment_views(&db, &raw_file_id, now).await;
        }

        let mut headers = cors::apply_cors(Headers::new())?;
        headers.set("Location", &tg_file_url)?;
        headers.set("Cache-Control", "public, max-age=86400, s-maxage=86400")?; // 24小时边缘重定向缓存
        return Ok(Response::empty()?.with_headers(headers).with_status(302));
    }

    // ⚡ 对于缩略图（size=thumb），依然完美保留你的 Cloudflare 边缘物理缓存机制！
    let cache_key = req.url()?.to_string();
    if let Ok(Some(cached_resp)) = Cache::default().get(&cache_key, true).await {
        return Ok(cached_resp);
    }

    let mut cdn_resp = tg::fetch_file_stream(&bot_token, &file_path).await?;
    let res_headers = cors::apply_cors(Headers::new())?;
    res_headers.set(
        "Cache-Control",
        "public, s-maxage=31536000, max-age=31536000, immutable",
    )?;
    res_headers.set("X-Content-Type-Options", "nosniff")?;

    let ext = filekey.split('.').last().unwrap_or("").to_lowercase();
    let mime_type = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    };
    res_headers.set("Content-Type", mime_type)?;

    let stream = cdn_resp.stream()?;
    let mut response = Response::from_stream(stream)?.with_headers(res_headers);

    if let Ok(resp_for_cache) = response.cloned() {
        let cache_key_clone = cache_key.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = Cache::default().put(cache_key_clone, resp_for_cache).await;
        });
    }

    Ok(response)
}

// 2. 物理永久彻底删除（原封不动保留你的逻辑）
pub async fn delete_file(req: Request, env: Env, imageid: String) -> Result<Response> {
    let db = env.d1("DB")?;
    let raw_bot_token = env.var("TG_Bot_Token")?.to_string();
    let bot_token = raw_bot_token.split(',').next().unwrap_or(&raw_bot_token).trim().to_string();
    let jwt_secret = env.var("JWT_SECRET")?.to_string();

    let claims = match crypto::authenticate(&req, &jwt_secret) {
        Ok(c) => c,
        Err(err) => return Response::error(err.to_string(), 401),
    };

    let img = match image::get_by_id(&db, &imageid).await? {
        Some(i) => i,
        None => return Response::error("图片不存在", 404),
    };

    if img.user_id != claims.sub && claims.username != "admin" {
        return Response::error("无权操作此资源", 403);
    }

    if let Some(msg_id) = img.message_id {
        let fallback_chat = env.var("TG_Chat_ID")
            .map(|v| v.to_string())
            .unwrap_or_default();
        let fallback_first = fallback_chat.split(',').next().unwrap_or("").to_string();

        let target_chat_id = img.chat_id.as_deref().unwrap_or(&fallback_first);
        if !target_chat_id.is_empty() {
            let _ = tg::delete_message(&bot_token, target_chat_id, msg_id).await;
        }
    }

    if let Ok(url) = req.url() {
        let origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""));
        for suffix in ["", "?size=thumb"] {
            let cache_key = format!("{}/file/{}{}", origin, imageid, suffix);
            let _ = Cache::default().delete(&cache_key, true).await;
        }
    }

    image::delete_physically(&db, &imageid).await?;
    crate::core::img_cache::invalidate(&imageid);

    let headers = cors::apply_cors(Headers::new())?;
    headers.set("Content-Type", "application/json")?;

    let response = Response::from_json(&serde_json::json!({
        "success": true,
        "message": "图片已物理永久删除，频道底图消息已销毁！"
    }))?.with_headers(headers);

    Ok(response)
}
