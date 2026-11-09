//! The system prompt (PRM Appendix B, `PROMPT_CHAT_SYSTEM_V1`) and the host note
//! (FR-PRM-44). Fixed in code and versioned (OQ-PRM-12).

/// The chat host of a turn (FR-PRM-43).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// Region edit (`rgn`).
    Rgn,
    /// New image (`gen`).
    Gen,
}

/// The version of the text below. Bump it when the wording changes.
pub const PROMPT_CHAT_SYSTEM_VERSION: u32 = 1;

/// Appendix B, common text.
pub const PROMPT_CHAT_SYSTEM_V1: &str = "You help a tabletop game master write a prompt for an image model. Each message may contain images, the model, its settings and the current prompt. Describe the desired result, not an instruction (\"a kitchen with a stone floor\", not \"add a kitchen\"). Keep within what the model and its settings can do. The listed helper texts, and the blend instruction when present, are appended automatically: do not repeat them. Put each suggested prompt in its own fenced block tagged `prompt`, at most three per reply. Keep the rest of the reply short. You cannot run jobs, see other files or change settings; the user decides what to send.";

/// Appendix B, Region edit paragraph.
pub const HOST_RGN_V1: &str = "The image shows the area being edited and its surroundings. The editable part is tinted magenta and outlined. Only that part will change, and it must blend into the rest.";

/// Appendix B, New image paragraph.
pub const HOST_GEN_V1: &str = "A new image will be generated from scratch. The first image, if marked as the structural sketch, fixes the layout. The other images are the reference images sent with the request.";

impl Host {
    /// The host's Appendix B paragraph.
    pub fn paragraph(self) -> &'static str {
        match self {
            Host::Rgn => HOST_RGN_V1,
            Host::Gen => HOST_GEN_V1,
        }
    }

    /// The name shown in the window and in the host note.
    pub fn label(self) -> &'static str {
        match self {
            Host::Rgn => "Region edit",
            Host::Gen => "New image",
        }
    }
}

/// The system prompt for a backend session whose first turn has `first_host` (FR-PRM-44).
pub fn system_prompt(first_host: Host) -> String {
    format!("{PROMPT_CHAT_SYSTEM_V1}\n\n{}", first_host.paragraph())
}

/// The one-line note prefixed to a user message whose host differs from the session's first.
pub fn host_note(host: Host) -> String {
    format!("[Now on: {}] {}", host.label(), host.paragraph())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prm_44_system_prompt_is_common_text_plus_first_host() {
        let rgn = system_prompt(Host::Rgn);
        assert!(rgn.starts_with(PROMPT_CHAT_SYSTEM_V1));
        assert!(rgn.ends_with(HOST_RGN_V1));
        assert!(!rgn.contains(HOST_GEN_V1));
        assert!(host_note(Host::Gen).starts_with("[Now on: New image] A new image"));
    }
}
