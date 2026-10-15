use worker::d1::D1Database;
use worker::wasm_bindgen::JsValue;
use worker::*;

// Option 值转 D1 绑定值：None 必须显式转 JS null，
// 直接 .into() 会把 None 变成 undefined，D1 拒绝绑定并抛 D1_TYPE_ERROR
pub fn opt_str(v: Option<&str>) -> JsValue {
    match v {
        Some(s) => JsValue::from_str(s),
        None => JsValue::null(),
    }
}

pub fn opt_i64(v: Option<i64>) -> JsValue {
    match v {
        Some(n) => JsValue::from_f64(n as f64),
        None => JsValue::null(),
    }
}

// i64 转 D1 绑定值：必须转 JS number（f64）。
// 直接 .into() 会变成 BigInt，而 D1 不支持 BigInt 类型
pub fn int_i64(v: i64) -> JsValue {
    JsValue::from_f64(v as f64)
}

// 在 Worker 运行期间，自动检测并自动创建所有必需的 SQL 数据表 (零摩擦冷启动自愈)
pub async fn init_db(db: &D1Database) -> Result<()> {
    // 1. 创建用户表
    db.prepare(
        "CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            email TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            avatar_url TEXT,
            bio TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );"
    ).run().await?;

    // 2. 创建相册表 (加入相册提取码密码支持)
    db.prepare(
        "CREATE TABLE IF NOT EXISTS albums (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            cover_url TEXT,
            password_hash TEXT,
            user_id TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(user_id) REFERENCES users(id)
        );"
    ).run().await?;

    // 3. 创建图片表 (加入缩略图 file_id、点击统计、最后访问统计字段)
    db.prepare(
"CREATE TABLE IF NOT EXISTS images (
            id TEXT PRIMARY KEY,
            file_id TEXT NOT NULL,
            file_name TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            mime_type TEXT NOT NULL,
            user_id TEXT NOT NULL,
            album_id TEXT,
            message_id INTEGER,
            chat_id TEXT,
            thumb_file_id TEXT,
            views INTEGER DEFAULT 0,
            last_accessed_at INTEGER,
            is_trash INTEGER DEFAULT 0,
            is_blocked INTEGER DEFAULT 0,
            uploaded_at INTEGER NOT NULL,
            FOREIGN KEY(user_id) REFERENCES users(id)
        );"
    ).run().await?;

    // 4. 创建标签表
    db.prepare(
        "CREATE TABLE IF NOT EXISTS tags (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            color TEXT,
            user_id TEXT NOT NULL,
            FOREIGN KEY(user_id) REFERENCES users(id),
            UNIQUE(name, user_id)
        );"
    ).run().await?;

    // 5. 创建图片-标签关联多对多表
    db.prepare(
        "CREATE TABLE IF NOT EXISTS image_tags (
            image_id TEXT NOT NULL,
            tag_id TEXT NOT NULL,
            PRIMARY KEY(image_id, tag_id),
            FOREIGN KEY(image_id) REFERENCES images(id) ON DELETE CASCADE,
            FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
        );"
    ).run().await?;

    // 6. 游客上传限速计数表（key = 'anonymous' 或用户 id）
    db.prepare(
        "CREATE TABLE IF NOT EXISTS upload_limits (
            key TEXT PRIMARY KEY,
            window_start INTEGER NOT NULL,
            count INTEGER NOT NULL
        );"
    ).run().await?;

    // 7. 温和迁移：老版本（JS 时代）的 D1 表可能缺新列，
    // CREATE TABLE IF NOT EXISTS 不会重建已存在的表，必须逐列补齐
    migrate(db).await?;

    // 8. 索引迁移：按全量查询模式建立显式索引（幂等），
    // 数据量增长后避免全表扫描（列表 / 公开画廊 / 相册 / 标签反向 / 关联表）
    migrate_indexes(db).await?;

    Ok(())
}

/// 检查表缺失的列并 ALTER TABLE ADD COLUMN 补齐（幂等，不触碰既有数据）
async fn migrate(db: &D1Database) -> Result<()> {
    let tables: [(&str, &[(&str, &str)]); 5] = [
        (
            "users",
            &[
                ("avatar_url", "TEXT"),
                ("bio", "TEXT"),
                ("created_at", "INTEGER"),
                ("updated_at", "INTEGER"),
            ],
        ),
        (
            "albums",
            &[
                ("description", "TEXT"),
                ("cover_url", "TEXT"),
                ("password_hash", "TEXT"),
            ],
        ),
(
            "images",
            &[
                ("album_id", "TEXT"),
                ("message_id", "INTEGER"),
                ("chat_id", "TEXT"),
                ("thumb_file_id", "TEXT"),
                ("views", "INTEGER DEFAULT 0"),
                ("last_accessed_at", "INTEGER"),
                ("is_trash", "INTEGER DEFAULT 0"),
                ("is_blocked", "INTEGER DEFAULT 0"),
            ],
        ),
        ("tags", &[("color", "TEXT")]),
        ("image_tags", &[]),
    ];

    for (table, columns) in tables {
        // PRAGMA 不支持参数绑定，表名来自代码常量，无注入风险
        let stmt = db.prepare(&format!("PRAGMA table_info({})", table));
        let rows = stmt.all().await?;
        let mut existing: std::collections::HashSet<String> = std::collections::HashSet::new();
        for row in rows.results::<serde_json::Value>()? {
            if let Some(name) = row.get("name").and_then(|v| v.as_str()) {
                existing.insert(name.to_string());
            }
        }

        for (col, ddl) in columns {
            if !existing.contains(*col) {
                let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, col, ddl);
                db.prepare(&sql).run().await?;
                console_log!("[migrate] {} ADD COLUMN {} {}", table, col, ddl);
            }
        }
    }

    Ok(())
}

/// 索引迁移：覆盖全部查询模式的显式索引（幂等，IF NOT EXISTS）
async fn migrate_indexes(db: &D1Database) -> Result<()> {
    let indexes: [(&str, &str); 6] = [
        (
            "idx_images_user_list",
            // 登录用户图片列表：WHERE user_id = ? AND is_trash = 0 ORDER BY uploaded_at DESC
            "CREATE INDEX IF NOT EXISTS idx_images_user_list ON images(user_id, is_trash, uploaded_at DESC)",
        ),
        (
            "idx_images_public",
            // 游客公开画廊：WHERE is_trash = 0 AND is_blocked = 0 ORDER BY uploaded_at DESC
            "CREATE INDEX IF NOT EXISTS idx_images_public ON images(is_trash, is_blocked, uploaded_at DESC)",
        ),
        (
            "idx_images_album",
            // 相册详情：WHERE album_id = ?
            "CREATE INDEX IF NOT EXISTS idx_images_album ON images(album_id)",
        ),
        (
            "idx_image_tags_tag",
            // 按标签拉图：JOIN image_tags ON ... WHERE it.tag_id = ?
            "CREATE INDEX IF NOT EXISTS idx_image_tags_tag ON image_tags(tag_id)",
        ),
        (
            "idx_tags_user",
            // 标签列表：WHERE user_id = ?
            "CREATE INDEX IF NOT EXISTS idx_tags_user ON tags(user_id)",
        ),
        (
            "idx_albums_user",
            // 相册列表：WHERE user_id = ?
            "CREATE INDEX IF NOT EXISTS idx_albums_user ON albums(user_id)",
        ),
    ];

    for (name, sql) in indexes {
        db.prepare(sql).run().await?;
        console_log!("[migrate] index {} ready", name);
    }

    Ok(())
}
