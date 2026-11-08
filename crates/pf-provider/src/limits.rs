//! The local limit backstop (FR-PRV-13, R20): RGN pre-checks with `input_too_large`; PRV
//! checks again in `estimate` and `submit` before any network call. References get the same
//! backstop (FR-PRV-33); REF enforces the limits first (FR-REF-18/21).

use crate::pixels::{png_size, sniff};
use crate::scenario::record::content_hash;
use crate::{Capabilities, JobRequest, ProviderError, ProviderResult, ReferenceImage};

fn invalid(field: &str) -> ProviderError {
    ProviderError::InvalidRequest {
        field: field.into(),
    }
}

/// The sent image must fit `input_px{min,max,multiple_of}` and `max_bytes`; the canonical
/// mask must be exactly the sent image's size (FR-PRV-12). Returns the image size.
pub fn check(caps: &Capabilities, request: &JobRequest) -> ProviderResult<(u32, u32)> {
    let (w, h) = png_size(&request.region_png).ok_or_else(|| invalid("image"))?;
    let px = caps.input_px;
    let fits = |s: u32| s >= px.min && s <= px.max && s.is_multiple_of(px.multiple_of.max(1));
    if !fits(w) || !fits(h) {
        return Err(invalid("image"));
    }
    if request.region_png.len() as u64 > caps.max_bytes {
        return Err(invalid("image"));
    }
    if png_size(&request.mask_png) != Some((w, h)) {
        return Err(invalid("mask"));
    }
    Ok((w, h))
}

/// A converted file over `max_bytes` → `invalid_request(field)`.
pub fn check_converted(caps: &Capabilities, bytes: &[u8], field: &str) -> ProviderResult<()> {
    if bytes.len() as u64 > caps.max_bytes {
        return Err(invalid(field));
    }
    Ok(())
}

/// Does `hash` (`sha256:<hex>` or bare hex) name these bytes?
pub fn hash_matches(r: &ReferenceImage) -> bool {
    let hex = r.hash.strip_prefix("sha256:").unwrap_or(&r.hash);
    hex.eq_ignore_ascii_case(&content_hash(&r.bytes))
}

/// The reference backstop (FR-PRV-33): every reference names a slot of the model, each slot's
/// count is within `min`–`max`, and every file is within `max_bytes`, sniffs as PNG, JPEG or
/// WebP and matches its hash. Anything else → `invalid_request(reference)`.
pub fn check_references(caps: &Capabilities, request: &JobRequest) -> ProviderResult<()> {
    for r in &request.references {
        let known = caps.slots.iter().any(|s| s.name == r.slot);
        if !known
            || r.bytes.len() as u64 > caps.max_bytes
            || sniff(&r.bytes).is_none()
            || !hash_matches(r)
        {
            return Err(invalid("reference"));
        }
    }
    for s in &caps.slots {
        let n = request
            .references
            .iter()
            .filter(|r| r.slot == s.name)
            .count();
        if n < s.min as usize || n > s.max as usize {
            return Err(invalid("reference"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::fixture_recording;
    use crate::scenario::normalise::normalise;
    use crate::scenario::overlay::{FAKE_ROWS, row_in};

    fn caps() -> Capabilities {
        let id = "fake-black-edit";
        let rec = fixture_recording(id, "get-model").unwrap();
        let body = &rec.exchanges[0].response.body;
        normalise(id, body, row_in(&FAKE_ROWS, id))
            .unwrap()
            .capabilities
    }

    fn request(references: Vec<ReferenceImage>) -> JobRequest {
        JobRequest {
            operation: crate::Operation::EditMasked,
            model_id: "fake-black-edit".into(),
            final_prompt: "p".into(),
            controls: Default::default(),
            references,
            region_png: vec![],
            mask_png: vec![],
            candidate_count: 1,
            seed: None,
        }
    }

    fn png_ref(hash: impl FnOnce(&str) -> String) -> ReferenceImage {
        let bytes = crate::fixtures::room_png(64);
        ReferenceImage {
            slot: "style_refs".into(),
            role: None,
            hash: hash(&content_hash(&bytes)),
            bytes,
        }
    }

    #[test]
    fn fr_prv_33_hash_forms_and_the_max_bytes_backstop() {
        let mut c = caps();
        for form in [
            |h: &str| format!("sha256:{h}"),
            |h: &str| h.to_owned(),
            |h: &str| h.to_ascii_uppercase(),
        ] {
            check_references(&c, &request(vec![png_ref(form)])).unwrap();
        }
        let r = png_ref(|h| h.to_owned());
        c.max_bytes = r.bytes.len() as u64 - 1;
        assert_eq!(
            check_references(&c, &request(vec![r])).unwrap_err(),
            invalid("reference")
        );
        // No references at all is fine when no slot has a `min`.
        check_references(&c, &request(vec![])).unwrap();
    }
}
