//! Shared PostgreSQL helpers for content-addressed artwork assets.

use sqlx::Transaction;

/// Parses checksum + content type from `artwork/{aa}/{bb}/{64hex}.{ext}`.
pub(crate) fn artwork_meta_from_storage_key(storage_key: &str) -> Option<(String, String)> {
    let rest = storage_key.strip_prefix("artwork/")?;
    let mut parts = rest.split('/');
    let _aa = parts.next()?;
    let _bb = parts.next()?;
    let file = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let (stem, ext) = file.rsplit_once('.')?;
    if stem.len() != 64 || !stem.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let content_type = match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => return None,
    };
    Some((stem.to_ascii_lowercase(), content_type.to_string()))
}

pub(crate) async fn upsert_artwork_asset(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    storage_key: &str,
    checksum_sha256: Option<&str>,
) -> Result<Option<uuid::Uuid>, sqlx::Error> {
    let (checksum, content_type) = match checksum_sha256 {
        Some(checksum) if checksum.len() == 64 && checksum.chars().all(|c| c.is_ascii_hexdigit()) => {
            let content_type = match storage_key.rsplit_once('.') {
                Some((_, "png" | "PNG")) => "image/png".to_string(),
                Some((_, "jpg" | "JPG" | "jpeg" | "JPEG")) => "image/jpeg".to_string(),
                _ => match artwork_meta_from_storage_key(storage_key) {
                    Some((_, content_type)) => content_type,
                    None => return Ok(None),
                },
            };
            (checksum.to_ascii_lowercase(), content_type)
        }
        _ => match artwork_meta_from_storage_key(storage_key) {
            Some(meta) => meta,
            None => return Ok(None),
        },
    };

    let id = sqlx::query_scalar::<_, uuid::Uuid>(
        r#"
            INSERT INTO artwork_assets (storage_key, content_type, checksum_sha256, size_bytes)
            VALUES ($1, $2, $3, 0)
            ON CONFLICT (storage_key) DO UPDATE
            SET content_type = EXCLUDED.content_type,
                checksum_sha256 = EXCLUDED.checksum_sha256,
                updated_at = NOW()
            RETURNING id
        "#,
    )
    .bind(storage_key)
    .bind(&content_type)
    .bind(&checksum)
    .fetch_one(&mut **tx)
    .await?;
    Ok(Some(id))
}
