//! The `pf-tiles` URI scheme, served by the core (EDT Appendix A). The Tauri layer only adapts
//! requests. This module holds the grammar, the response type and the whole-image registry
//! (`/i/<key>.<ext>`); tile pyramids (`/t/…`) and the serving entry point are in `tiles`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Where a registered image's bytes come from.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageBody {
    /// A file the core resolved itself (an original, a layer file, a cached copy).
    File(PathBuf),
    /// In-memory bytes: a transient image (FR-EDT-31).
    Bytes(Arc<Vec<u8>>),
    /// A `library` key (OQ-PRJ-16, BL-16): `src` fitted within `max_side`, made on the first
    /// request and cached at `dst`. A source that already fits is served as it is.
    Copy {
        src: PathBuf,
        dst: PathBuf,
        max_side: u32,
    },
}

/// An image registered for the open asset project.
#[derive(Debug, Clone, PartialEq)]
pub struct ServedImage {
    pub body: ImageBody,
    pub mime: &'static str,
    pub ext: &'static str,
}

impl ServedImage {
    /// A registered file.
    pub fn file(path: PathBuf, mime: &'static str, ext: &'static str) -> Self {
        ServedImage {
            body: ImageBody::File(path),
            mime,
            ext,
        }
    }
}

/// Keys → images. Keys are unregistered when the project closes (then 404).
#[derive(Debug, Default)]
pub struct ImageRegistry {
    images: HashMap<String, ServedImage>,
}

impl ImageRegistry {
    pub fn register(&mut self, key: String, image: ServedImage) {
        self.images.insert(key, image);
    }

    pub fn remove(&mut self, key: &str) -> Option<ServedImage> {
        self.images.remove(key)
    }

    pub fn clear(&mut self) {
        self.images.clear();
    }

    pub fn get(&self, key: &str) -> Option<&ServedImage> {
        self.images.get(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = String> + '_ {
        self.images.keys().cloned()
    }

    /// Points every file under the folder `old` at the same file under `new` (the open
    /// project's folder was renamed, FR-PRJ-56). Keys stay, so URLs the UI holds keep working.
    pub fn rebase(&mut self, old: &std::path::Path, new: &std::path::Path) {
        let moved = |p: &mut PathBuf| {
            if let Ok(rest) = p.strip_prefix(old) {
                *p = new.join(rest);
            }
        };
        for img in self.images.values_mut() {
            match &mut img.body {
                ImageBody::File(p) => moved(p),
                ImageBody::Copy { src, .. } => moved(src),
                ImageBody::Bytes(_) => {}
            }
        }
    }
}

/// A scheme response, independent of any HTTP library.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemeResponse {
    pub status: u16,
    pub content_type: Option<&'static str>,
    pub cache_immutable: bool,
    pub body: Vec<u8>,
}

impl SchemeResponse {
    pub fn status(status: u16) -> Self {
        SchemeResponse {
            status,
            content_type: None,
            cache_immutable: false,
            body: Vec::new(),
        }
    }
}

/// A parsed request path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemePath {
    Image {
        key: String,
        ext: String,
    },
    Tile {
        key: String,
        level: u32,
        col: u32,
        row: u32,
        ext: String,
    },
}

fn valid_key(k: &str) -> bool {
    (1..=80).contains(&k.len())
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn decimal(s: &str) -> Option<u32> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// Parses `/i/<key>.<png|webp|jpg>` and `/t/<key>/<level>/<col>_<row>.<jpg|webp>`. Anything
/// else (traversal, encoded dots, absolute paths, bad grammar) is `None` → 400.
pub fn parse_path(path: &str) -> Option<SchemePath> {
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let rest = path.strip_prefix('/')?;
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        ["i", file] => {
            let (key, ext) = file.rsplit_once('.')?;
            (valid_key(key) && matches!(ext, "png" | "webp" | "jpg")).then(|| SchemePath::Image {
                key: key.into(),
                ext: ext.into(),
            })
        }
        ["t", key, level, file] => {
            let (cr, ext) = file.rsplit_once('.')?;
            let (col, row) = cr.split_once('_')?;
            (valid_key(key) && matches!(ext, "jpg" | "webp")).then_some(())?;
            Some(SchemePath::Tile {
                key: (*key).into(),
                level: decimal(level)?,
                col: decimal(col)?,
                row: decimal(row)?,
                ext: ext.into(),
            })
        }
        _ => None,
    }
}

/// A 200 response with immutable caching (keys never change meaning, Appendix A).
pub fn ok(content_type: &'static str, body: Vec<u8>) -> SchemeResponse {
    SchemeResponse {
        status: 200,
        content_type: Some(content_type),
        cache_immutable: true,
        body,
    }
}

/// Reads a registered image (outside any lock). Never maps a URL to a file path itself.
pub fn read_image(key: &str, img: &ServedImage) -> SchemeResponse {
    match &img.body {
        ImageBody::Bytes(b) => ok(img.mime, b.as_ref().clone()),
        ImageBody::Copy { src, dst, max_side } => {
            crate::tiles::serve_copy(key, src, dst, *max_side)
        }
        ImageBody::File(path) => match std::fs::read(path) {
            Ok(body) => ok(img.mime, body),
            Err(e) => {
                tracing::warn!(key, error = %e, "pf-tiles: registered image unreadable");
                SchemeResponse::status(404)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_edt_09_uri_grammar() {
        assert_eq!(
            parse_path("/i/01ABC_x-y.png"),
            Some(SchemePath::Image {
                key: "01ABC_x-y".into(),
                ext: "png".into()
            })
        );
        assert_eq!(
            parse_path("/t/k/3/10_2.jpg"),
            Some(SchemePath::Tile {
                key: "k".into(),
                level: 3,
                col: 10,
                row: 2,
                ext: "jpg".into()
            })
        );
        for bad in [
            "",
            "/",
            "/i/",
            "/i/.png",
            "/i/a.gif",
            "/i/../x.png",
            "/i/%2e%2e.png",
            "/i/a/b.png",
            "/t/k/x/1_2.jpg",
            "/t/k/1/1-2.jpg",
            "/t/k/+1/1_2.jpg",
            "C:/Windows/x.png",
            "/x/a.png",
            "/t/../0/0_0.jpg",
            "/t/%2e%2e/0/0_0.jpg",
            "/t/k/0/0_0.png",
            "/t/k/0/../0_0.jpg",
            "/t/k/0/1_2_3.jpg",
            "/t/k/0/-1_0.jpg",
            "/t/k/0/0_0",
            "/t/k/0/0_0.jpg/x",
            "/t/C:/0/0_0.jpg",
            "//t/k/0/0_0.jpg",
        ] {
            assert_eq!(parse_path(bad), None, "{bad}");
        }
        assert_eq!(parse_path(&format!("/i/{}.png", "a".repeat(81))), None);
    }
}
