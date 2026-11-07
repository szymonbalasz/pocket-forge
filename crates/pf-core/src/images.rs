//! The `pf-tiles` URI scheme, served by the core (EDT Appendix A). The Tauri layer only adapts
//! requests. Skeleton scope: `/i/<key>.<ext>` serves a registered whole image; the tile pyramid
//! (`/t/…`) is EDT slice 1.

use std::collections::HashMap;
use std::path::PathBuf;

/// An image registered for the open asset project.
#[derive(Debug, Clone, PartialEq)]
pub struct ServedImage {
    pub path: PathBuf,
    pub mime: &'static str,
    pub ext: &'static str,
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

    pub fn clear(&mut self) {
        self.images.clear();
    }

    pub fn get(&self, key: &str) -> Option<&ServedImage> {
        self.images.get(key)
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
    fn status(status: u16) -> Self {
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

/// Serves a request path from the registry. Never maps a URL to a file path itself.
pub fn serve(registry: &ImageRegistry, path: &str) -> SchemeResponse {
    match parse_path(path) {
        None => SchemeResponse::status(400),
        Some(SchemePath::Tile { .. }) => SchemeResponse::status(404),
        Some(SchemePath::Image { key, ext }) => match registry.get(&key) {
            Some(img) if img.ext == ext => match std::fs::read(&img.path) {
                Ok(body) => SchemeResponse {
                    status: 200,
                    content_type: Some(img.mime),
                    cache_immutable: true,
                    body,
                },
                Err(e) => {
                    tracing::warn!(key, error = %e, "pf-tiles: registered image unreadable");
                    SchemeResponse::status(404)
                }
            },
            _ => SchemeResponse::status(404),
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
        ] {
            assert_eq!(parse_path(bad), None, "{bad}");
        }
        assert_eq!(parse_path(&format!("/i/{}.png", "a".repeat(81))), None);
    }

    #[test]
    fn fr_edt_09_serves_only_registered_keys() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.png");
        std::fs::write(&file, b"png-bytes").unwrap();
        let mut reg = ImageRegistry::default();
        reg.register(
            "k1".into(),
            ServedImage {
                path: file,
                mime: "image/png",
                ext: "png",
            },
        );
        let ok = serve(&reg, "/i/k1.png");
        assert_eq!((ok.status, ok.body.as_slice()), (200, &b"png-bytes"[..]));
        assert!(ok.cache_immutable);
        assert_eq!(serve(&reg, "/i/k1.webp").status, 404);
        assert_eq!(serve(&reg, "/i/k2.png").status, 404);
        assert_eq!(serve(&reg, "/i/../a.png").status, 400);
        reg.clear();
        assert_eq!(serve(&reg, "/i/k1.png").status, 404);
    }
}
