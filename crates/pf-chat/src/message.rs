//! The stream-json stdin message for one turn (§10): one JSON line with the image blocks
//! first, then one text block.

use serde_json::{Value, json};

use crate::base64;

/// One image to attach (FR-PRM-25: JPEG after downscaling).
#[derive(Debug, Clone, Copy)]
pub struct ImageBlock<'a> {
    pub media_type: &'a str,
    pub data: &'a [u8],
}

/// The user message as one JSON line, without the trailing newline. JSON escaping keeps
/// newlines in `text` out of the line.
pub fn user_message_line(images: &[ImageBlock<'_>], text: &str) -> String {
    let mut content: Vec<Value> = images
        .iter()
        .map(|img| {
            json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": img.media_type,
                    "data": base64::encode(img.data),
                }
            })
        })
        .collect();
    content.push(json!({ "type": "text", "text": text }));
    json!({
        "type": "user",
        "message": { "role": "user", "content": content },
        "parent_tool_use_id": null,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prm_s10_stdin_message_shape() {
        let jpeg = [0xffu8, 0xd8, 0xff];
        let line = user_message_line(
            &[ImageBlock {
                media_type: "image/jpeg",
                data: &jpeg,
            }],
            "line one\nline two",
        );
        assert!(!line.contains('\n'));
        let v: Value = serde_json::from_str(&line).unwrap_or_default();
        assert_eq!(v["type"], "user");
        assert_eq!(v["parent_tool_use_id"], Value::Null);
        assert_eq!(v["message"]["role"], "user");
        let c = &v["message"]["content"];
        assert_eq!(c[0]["type"], "image");
        assert_eq!(c[0]["source"]["type"], "base64");
        assert_eq!(c[0]["source"]["media_type"], "image/jpeg");
        assert_eq!(c[0]["source"]["data"], "/9j/");
        assert_eq!(c[1]["type"], "text");
        assert_eq!(c[1]["text"], "line one\nline two");
    }
}
