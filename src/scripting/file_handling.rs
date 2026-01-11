//! File & Media Handling for Lua Scripts
//!
//! This module provides file upload, metadata, and image processing
//! functions for Lua scripts in SoliDB using the blob storage system.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use image::{DynamicImage, GenericImageView, ImageFormat};
use mlua::{Function, Lua, Result as LuaResult, Table, Value as LuaValue};
use serde_json::Value as JsonValue;
use std::io::Cursor;
use std::sync::Arc;

use crate::error::DbError;
use crate::storage::StorageEngine;

/// Default chunk size for blob storage (1MB)
const CHUNK_SIZE: usize = 1024 * 1024;

/// Files collection name
const FILES_COLLECTION: &str = "_files";

/// Detect MIME type from file extension
fn mime_from_extension(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Detect MIME type from magic bytes
fn mime_from_magic(data: &[u8]) -> Option<&'static str> {
    if data.len() < 4 {
        return None;
    }

    // Check magic bytes
    match &data[..4] {
        [0xFF, 0xD8, 0xFF, _] => Some("image/jpeg"),
        [0x89, 0x50, 0x4E, 0x47] => Some("image/png"),
        [0x47, 0x49, 0x46, 0x38] => Some("image/gif"),
        [0x52, 0x49, 0x46, 0x46] if data.len() >= 12 && &data[8..12] == b"WEBP" => {
            Some("image/webp")
        }
        [0x25, 0x50, 0x44, 0x46] => Some("application/pdf"),
        [0x50, 0x4B, 0x03, 0x04] => Some("application/zip"),
        _ => None,
    }
}

/// Get extension from MIME type
fn extension_from_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        _ => "bin",
    }
}

/// Ensure the _files collection exists as a blob collection
fn ensure_files_collection(storage: &StorageEngine, db_name: &str) -> Result<(), mlua::Error> {
    let database = storage
        .get_database(db_name)
        .map_err(|e| mlua::Error::RuntimeError(format!("Failed to access database: {}", e)))?;

    // Check if _files collection exists
    match database.get_collection(FILES_COLLECTION) {
        Ok(coll) => {
            // Verify it's a blob collection
            if coll.get_type() != "blob" {
                return Err(mlua::Error::RuntimeError(format!(
                    "Collection '{}' exists but is not a blob collection",
                    FILES_COLLECTION
                )));
            }
        }
        Err(DbError::CollectionNotFound(_)) => {
            // Create as blob collection
            database
                .create_collection(FILES_COLLECTION.to_string(), Some("blob".to_string()))
                .map_err(|e| {
                    mlua::Error::RuntimeError(format!("Failed to create _files collection: {}", e))
                })?;
        }
        Err(e) => {
            return Err(mlua::Error::RuntimeError(format!(
                "Failed to check _files collection: {}",
                e
            )));
        }
    }

    Ok(())
}

/// Create solidb.upload(data, options) -> file info function
/// Options: { filename, directory, overwrite }
/// Stores files in the _files blob collection
pub fn create_upload_function(
    lua: &Lua,
    storage: Arc<StorageEngine>,
    db_name: String,
) -> LuaResult<Function> {
    lua.create_function(move |lua, (data, options): (LuaValue, Option<Table>)| {
        // Extract binary data (base64 string or raw bytes)
        let bytes: Vec<u8> = match data {
            LuaValue::String(s) => {
                let s_bytes = s.as_bytes().to_vec();
                // Try to decode as base64, otherwise use as raw bytes
                BASE64.decode(&s_bytes).unwrap_or(s_bytes)
            }
            LuaValue::Table(t) => {
                // Array of bytes
                let mut bytes = Vec::new();
                for i in 1..=t.len()? {
                    if let Ok(b) = t.get::<i64>(i) {
                        bytes.push(b as u8);
                    }
                }
                bytes
            }
            _ => {
                return Err(mlua::Error::RuntimeError(
                    "upload: data must be a base64 string or byte array".to_string(),
                ))
            }
        };

        if bytes.is_empty() {
            return Err(mlua::Error::RuntimeError("upload: empty data".to_string()));
        }

        // Parse options
        let filename = options
            .as_ref()
            .and_then(|o| o.get::<String>("filename").ok());

        let directory = options
            .as_ref()
            .and_then(|o| o.get::<String>("directory").ok());

        // Generate file key (UUID v7 for time-ordering)
        let file_key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();

        // Determine filename and extension
        let (safe_filename, ext) = if let Some(ref fname) = filename {
            let sanitized: String = fname
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
                .collect();
            let extension = sanitized.rsplit('.').next().unwrap_or("bin").to_lowercase();
            (sanitized, extension)
        } else {
            let ext = mime_from_magic(&bytes)
                .map(extension_from_mime)
                .unwrap_or("bin");
            (format!("{}.{}", file_key, ext), ext.to_string())
        };

        if safe_filename.is_empty() {
            return Err(mlua::Error::RuntimeError(
                "upload: invalid filename".to_string(),
            ));
        }

        // Detect MIME type
        let mime_type = mime_from_magic(&bytes).unwrap_or_else(|| mime_from_extension(&ext));

        // Ensure _files collection exists
        ensure_files_collection(&storage, &db_name)?;

        // Get collection
        let database = storage
            .get_database(&db_name)
            .map_err(|e| mlua::Error::RuntimeError(format!("upload: {}", e)))?;
        let collection = database
            .get_collection(FILES_COLLECTION)
            .map_err(|e| mlua::Error::RuntimeError(format!("upload: {}", e)))?;

        // Store blob chunks
        let total_size = bytes.len();
        let mut chunk_index = 0u32;
        for chunk in bytes.chunks(CHUNK_SIZE) {
            collection
                .put_blob_chunk(&file_key, chunk_index, chunk)
                .map_err(|e| {
                    mlua::Error::RuntimeError(format!("upload: failed to store chunk: {}", e))
                })?;
            chunk_index += 1;
        }

        // Build path (directory/filename or just filename)
        let path = if let Some(ref dir) = directory {
            let safe_dir = dir.replace("..", "").replace("//", "/");
            format!("{}/{}", safe_dir.trim_matches('/'), safe_filename)
        } else {
            safe_filename.clone()
        };

        // Create metadata document
        let mut metadata = serde_json::Map::new();
        metadata.insert("_key".to_string(), JsonValue::String(file_key.clone()));
        metadata.insert("path".to_string(), JsonValue::String(path.clone()));
        metadata.insert(
            "filename".to_string(),
            JsonValue::String(safe_filename.clone()),
        );
        metadata.insert("size".to_string(), JsonValue::Number(total_size.into()));
        metadata.insert(
            "mime_type".to_string(),
            JsonValue::String(mime_type.to_string()),
        );
        metadata.insert("chunks".to_string(), JsonValue::Number(chunk_index.into()));
        metadata.insert(
            "created_at".to_string(),
            JsonValue::String(chrono::Utc::now().to_rfc3339()),
        );

        if let Some(ref dir) = directory {
            metadata.insert("directory".to_string(), JsonValue::String(dir.clone()));
        }

        // Add image dimensions if applicable
        if mime_type.starts_with("image/") {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let (width, height) = img.dimensions();
                metadata.insert("width".to_string(), JsonValue::Number(width.into()));
                metadata.insert("height".to_string(), JsonValue::Number(height.into()));
            }
        }

        // Store metadata document
        collection
            .insert(JsonValue::Object(metadata))
            .map_err(|e| {
                mlua::Error::RuntimeError(format!("upload: failed to store metadata: {}", e))
            })?;

        // Return file info
        let result = lua.create_table()?;
        result.set("key", file_key)?;
        result.set("path", path)?;
        result.set("filename", safe_filename)?;
        result.set("size", total_size)?;
        result.set("mime_type", mime_type)?;
        result.set("chunks", chunk_index)?;

        Ok(result)
    })
}

/// Create solidb.file_info(key) -> file metadata function
pub fn create_file_info_function(
    lua: &Lua,
    storage: Arc<StorageEngine>,
    db_name: String,
) -> LuaResult<Function> {
    lua.create_function(move |lua, key: String| {
        // Get collection
        let database = storage
            .get_database(&db_name)
            .map_err(|e| mlua::Error::RuntimeError(format!("file_info: {}", e)))?;

        let collection = match database.get_collection(FILES_COLLECTION) {
            Ok(c) => c,
            Err(_) => {
                return Err(mlua::Error::RuntimeError(format!(
                    "file_info: file not found: {}",
                    key
                )))
            }
        };

        // Get metadata document
        let doc = collection.get(&key).map_err(|_| {
            mlua::Error::RuntimeError(format!("file_info: file not found: {}", key))
        })?;

        // Convert to Lua table
        let result = lua.create_table()?;

        // Add standard fields
        result.set("_key", doc.key.as_str())?;
        result.set("_id", doc.id.as_str())?;

        // Add data fields
        if let JsonValue::Object(obj) = &doc.data {
            for (k, v) in obj {
                match v {
                    JsonValue::String(s) => {
                        result.set(k.as_str(), s.as_str())?;
                    }
                    JsonValue::Number(n) => {
                        if let Some(i) = n.as_u64() {
                            result.set(k.as_str(), i)?;
                        } else if let Some(f) = n.as_f64() {
                            result.set(k.as_str(), f)?;
                        }
                    }
                    JsonValue::Bool(b) => {
                        result.set(k.as_str(), *b)?;
                    }
                    _ => {}
                }
            }
        }

        Ok(result)
    })
}

/// Create solidb.file_read(key) -> base64 string function
pub fn create_file_read_function(
    lua: &Lua,
    storage: Arc<StorageEngine>,
    db_name: String,
) -> LuaResult<Function> {
    lua.create_function(move |_lua, key: String| {
        // Get collection
        let database = storage
            .get_database(&db_name)
            .map_err(|e| mlua::Error::RuntimeError(format!("file_read: {}", e)))?;

        let collection = match database.get_collection(FILES_COLLECTION) {
            Ok(c) => c,
            Err(_) => {
                return Err(mlua::Error::RuntimeError(format!(
                    "file_read: file not found: {}",
                    key
                )))
            }
        };

        // Get metadata to know chunk count
        let doc = collection.get(&key).map_err(|_| {
            mlua::Error::RuntimeError(format!("file_read: file not found: {}", key))
        })?;

        let chunk_count = doc.get("chunks").and_then(|v| v.as_u64()).unwrap_or(1) as u32;

        // Read all chunks
        let mut data = Vec::new();
        for i in 0..chunk_count {
            if let Ok(Some(chunk)) = collection.get_blob_chunk(&key, i) {
                data.extend(chunk);
            }
        }

        // Return as base64
        Ok(BASE64.encode(&data))
    })
}

/// Create solidb.file_delete(key) -> boolean function
pub fn create_file_delete_function(
    lua: &Lua,
    storage: Arc<StorageEngine>,
    db_name: String,
) -> LuaResult<Function> {
    lua.create_function(move |_lua, key: String| {
        // Get collection
        let database = storage
            .get_database(&db_name)
            .map_err(|e| mlua::Error::RuntimeError(format!("file_delete: {}", e)))?;

        let collection = match database.get_collection(FILES_COLLECTION) {
            Ok(c) => c,
            Err(_) => return Ok(false),
        };

        // Check if file exists
        match collection.get(&key) {
            Ok(_) => {
                // Delete blob chunks
                collection
                    .delete_blob_data(&key)
                    .map_err(|e| mlua::Error::RuntimeError(format!("file_delete: {}", e)))?;

                // Delete metadata document
                collection
                    .delete(&key)
                    .map_err(|e| mlua::Error::RuntimeError(format!("file_delete: {}", e)))?;

                Ok(true)
            }
            Err(_) => Ok(false),
        }
    })
}

/// Create solidb.file_list(options?) -> array of file info
/// Options: { directory, limit, offset }
pub fn create_file_list_function(
    lua: &Lua,
    storage: Arc<StorageEngine>,
    db_name: String,
) -> LuaResult<Function> {
    lua.create_function(move |lua, options: Option<Table>| {
        // Get collection
        let database = storage
            .get_database(&db_name)
            .map_err(|e| mlua::Error::RuntimeError(format!("file_list: {}", e)))?;

        let collection = match database.get_collection(FILES_COLLECTION) {
            Ok(c) => c,
            Err(_) => {
                // Collection doesn't exist, return empty array
                return Ok(lua.create_table()?);
            }
        };

        // Parse options
        let directory = options
            .as_ref()
            .and_then(|o| o.get::<String>("directory").ok());
        let limit = options
            .as_ref()
            .and_then(|o| o.get::<usize>("limit").ok())
            .unwrap_or(100);
        let offset = options
            .as_ref()
            .and_then(|o| o.get::<usize>("offset").ok())
            .unwrap_or(0);

        // Scan all documents
        let docs = collection.scan(Some(limit + offset));

        let result = lua.create_table()?;
        let mut index = 1;
        let mut skipped = 0;

        for doc in docs {
            // Filter by directory if specified
            if let Some(ref dir) = directory {
                let doc_dir = doc
                    .get("directory")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                if doc_dir != *dir {
                    continue;
                }
            }

            // Handle offset
            if skipped < offset {
                skipped += 1;
                continue;
            }

            // Stop at limit
            if index > limit {
                break;
            }

            // Convert to Lua table
            let file_info = lua.create_table()?;
            file_info.set("_key", doc.key.as_str())?;
            file_info.set("_id", doc.id.as_str())?;

            if let JsonValue::Object(obj) = &doc.data {
                for (k, v) in obj {
                    match v {
                        JsonValue::String(s) => {
                            file_info.set(k.as_str(), s.as_str())?;
                        }
                        JsonValue::Number(n) => {
                            if let Some(i) = n.as_u64() {
                                file_info.set(k.as_str(), i)?;
                            } else if let Some(f) = n.as_f64() {
                                file_info.set(k.as_str(), f)?;
                            }
                        }
                        JsonValue::Bool(b) => {
                            file_info.set(k.as_str(), *b)?;
                        }
                        _ => {}
                    }
                }
            }

            result.set(index, file_info)?;
            index += 1;
        }

        Ok(result)
    })
}

/// Create solidb.image_process(data, operations) -> processed image (base64)
/// Operations: { resize: {width, height}, crop: {x, y, width, height},
///               rotate: 90|180|270, flip: "horizontal"|"vertical",
///               format: "jpeg"|"png"|"webp", quality: 1-100 }
pub fn create_image_process_function(
    lua: &Lua,
    storage: Arc<StorageEngine>,
    db_name: String,
) -> LuaResult<Function> {
    lua.create_function(move |lua, (data, operations): (LuaValue, Table)| {
        // Parse input data (base64 string, file key, or byte array)
        let bytes: Vec<u8> = match data {
            LuaValue::String(s) => {
                let s_str = s
                    .to_str()
                    .map_err(|e| mlua::Error::RuntimeError(format!("image_process: {}", e)))?;

                // Try as file key first (UUID format)
                if s_str.len() == 36 && s_str.chars().filter(|c| *c == '-').count() == 4 {
                    // Looks like a UUID, try to load from blob storage
                    let file_key = s_str.to_string();
                    if let Ok(database) = storage.get_database(&db_name) {
                        if let Ok(collection) = database.get_collection(FILES_COLLECTION) {
                            if let Ok(doc) = collection.get(&file_key) {
                                let chunk_count =
                                    doc.get("chunks").and_then(|v| v.as_u64()).unwrap_or(1) as u32;

                                let mut file_data = Vec::new();
                                for i in 0..chunk_count {
                                    if let Ok(Some(chunk)) = collection.get_blob_chunk(&file_key, i)
                                    {
                                        file_data.extend(chunk);
                                    }
                                }

                                if !file_data.is_empty() {
                                    return process_image(lua, file_data, operations);
                                }
                            }
                        }
                    }
                }

                // Try as base64
                BASE64.decode(s_str.as_bytes()).map_err(|e| {
                    mlua::Error::RuntimeError(format!("image_process: invalid data: {}", e))
                })?
            }
            _ => {
                return Err(mlua::Error::RuntimeError(
                    "image_process: data must be a base64 string or file key".to_string(),
                ))
            }
        };

        process_image(lua, bytes, operations)
    })
}

/// Process image with given operations
fn process_image(lua: &Lua, bytes: Vec<u8>, operations: Table) -> LuaResult<Table> {
    // Load image
    let mut img = image::load_from_memory(&bytes).map_err(|e| {
        mlua::Error::RuntimeError(format!("image_process: failed to load image: {}", e))
    })?;

    // Apply operations
    // Resize
    if let Ok(resize) = operations.get::<Table>("resize") {
        let width: Option<u32> = resize.get("width").ok();
        let height: Option<u32> = resize.get("height").ok();

        match (width, height) {
            (Some(w), Some(h)) => {
                img = img.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
            }
            (Some(w), None) => {
                // Maintain aspect ratio
                let ratio = w as f64 / img.width() as f64;
                let h = (img.height() as f64 * ratio) as u32;
                img = img.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
            }
            (None, Some(h)) => {
                // Maintain aspect ratio
                let ratio = h as f64 / img.height() as f64;
                let w = (img.width() as f64 * ratio) as u32;
                img = img.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
            }
            _ => {}
        }
    }

    // Thumbnail (resize to fit within bounds)
    if let Ok(thumb) = operations.get::<Table>("thumbnail") {
        let width: u32 = thumb.get("width").unwrap_or(100);
        let height: u32 = thumb.get("height").unwrap_or(100);
        img = img.thumbnail(width, height);
    }

    // Crop
    if let Ok(crop) = operations.get::<Table>("crop") {
        let x: u32 = crop.get("x").unwrap_or(0);
        let y: u32 = crop.get("y").unwrap_or(0);
        let width: u32 = crop.get("width").unwrap_or(img.width());
        let height: u32 = crop.get("height").unwrap_or(img.height());

        // Validate bounds
        if x + width <= img.width() && y + height <= img.height() {
            img = img.crop_imm(x, y, width, height);
        }
    }

    // Rotate
    if let Ok(rotate) = operations.get::<i32>("rotate") {
        img = match rotate {
            90 => img.rotate90(),
            180 => img.rotate180(),
            270 => img.rotate270(),
            _ => img,
        };
    }

    // Flip
    if let Ok(flip) = operations.get::<String>("flip") {
        img = match flip.as_str() {
            "horizontal" | "h" => img.fliph(),
            "vertical" | "v" => img.flipv(),
            _ => img,
        };
    }

    // Grayscale
    if operations.get::<bool>("grayscale").unwrap_or(false) {
        img = DynamicImage::ImageLuma8(img.to_luma8());
    }

    // Blur
    if let Ok(sigma) = operations.get::<f32>("blur") {
        img = img.blur(sigma);
    }

    // Brightness adjustment (-100 to 100)
    if let Ok(brightness) = operations.get::<i32>("brightness") {
        let factor = brightness.clamp(-100, 100);
        img = img.brighten(factor);
    }

    // Contrast adjustment (-100 to 100)
    if let Ok(contrast) = operations.get::<f32>("contrast") {
        let factor = contrast.clamp(-100.0, 100.0);
        img = img.adjust_contrast(factor);
    }

    // Output format
    let format_str: String = operations
        .get("format")
        .unwrap_or_else(|_| "png".to_string());
    let quality: u8 = operations.get("quality").unwrap_or(85);

    let format = match format_str.to_lowercase().as_str() {
        "jpeg" | "jpg" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        "webp" => ImageFormat::WebP,
        "gif" => ImageFormat::Gif,
        _ => ImageFormat::Png,
    };

    // Encode output
    let mut output = Cursor::new(Vec::new());

    match format {
        ImageFormat::Jpeg => {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality);
            img.write_with_encoder(encoder).map_err(|e| {
                mlua::Error::RuntimeError(format!("image_process: encode failed: {}", e))
            })?;
        }
        ImageFormat::WebP => {
            // WebP encoding
            img.write_to(&mut output, format).map_err(|e| {
                mlua::Error::RuntimeError(format!("image_process: encode failed: {}", e))
            })?;
        }
        _ => {
            img.write_to(&mut output, format).map_err(|e| {
                mlua::Error::RuntimeError(format!("image_process: encode failed: {}", e))
            })?;
        }
    }

    let output_bytes = output.into_inner();

    // Build result
    let result = lua.create_table()?;
    result.set("data", BASE64.encode(&output_bytes))?;
    result.set("size", output_bytes.len())?;
    result.set("width", img.width())?;
    result.set("height", img.height())?;
    result.set("format", format_str)?;
    result.set(
        "mime_type",
        match format {
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Png => "image/png",
            ImageFormat::WebP => "image/webp",
            ImageFormat::Gif => "image/gif",
            _ => "image/png",
        },
    )?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_from_extension() {
        assert_eq!(mime_from_extension("jpg"), "image/jpeg");
        assert_eq!(mime_from_extension("PNG"), "image/png");
        assert_eq!(mime_from_extension("pdf"), "application/pdf");
        assert_eq!(mime_from_extension("unknown"), "application/octet-stream");
    }

    #[test]
    fn test_mime_from_magic() {
        // JPEG magic bytes
        let jpeg_data = [0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(mime_from_magic(&jpeg_data), Some("image/jpeg"));

        // PNG magic bytes
        let png_data = [0x89, 0x50, 0x4E, 0x47];
        assert_eq!(mime_from_magic(&png_data), Some("image/png"));

        // Unknown
        let unknown = [0x00, 0x00, 0x00, 0x00];
        assert_eq!(mime_from_magic(&unknown), None);
    }

    #[test]
    fn test_extension_from_mime() {
        assert_eq!(extension_from_mime("image/jpeg"), "jpg");
        assert_eq!(extension_from_mime("image/png"), "png");
        assert_eq!(extension_from_mime("application/pdf"), "pdf");
        assert_eq!(extension_from_mime("unknown/type"), "bin");
    }
}
