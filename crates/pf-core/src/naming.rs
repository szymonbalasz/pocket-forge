//! Folder names: project names (FR-PRJ-44), collision suffixes (FR-PRJ-08) and iteration folder
//! names (FR-RGN-39).

/// Longest project name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// Longest iteration slug before the name rules, in characters (FR-RGN-39).
pub const MAX_SLUG_CHARS: usize = 32;

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn trim_name(s: &str) -> &str {
    // Trailing dots and spaces are invalid on Windows. Leading dots would make the folder
    // hidden (and look like the app's own `.pforge` and temp folders), so they go too.
    s.trim_end_matches(['.', ' '])
        .trim_start_matches(['.', ' '])
}

fn is_reserved(s: &str) -> bool {
    // `CON`, and `CON.anything`, are reserved whatever the case.
    let base = s.split('.').next().unwrap_or_default().trim_end();
    RESERVED.iter().any(|r| r.eq_ignore_ascii_case(base))
}

/// Applies the FR-PRJ-44 rules: `<>:"/\|?*` and control characters → `-`, leading dots and
/// spaces and trailing dots and spaces trimmed, reserved Windows names suffixed with `_`,
/// at most 64 characters; empty → `untitled`.
pub fn sanitise_name(raw: &str) -> String {
    let replaced: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let mut name: String = trim_name(&replaced).chars().take(MAX_NAME_CHARS).collect();
    name = trim_name(&name).to_owned();
    if is_reserved(&name) {
        if name.chars().count() >= MAX_NAME_CHARS {
            name = name.chars().take(MAX_NAME_CHARS - 1).collect();
        }
        name.push('_');
    }
    if name.is_empty() {
        name = "untitled".into();
    }
    name
}

/// True if `raw` is already a valid name (what `project_rename` accepts as typed).
pub fn is_valid_name(raw: &str) -> bool {
    !raw.is_empty() && sanitise_name(raw) == raw
}

/// `name` for n < 2, else `name (n)`, keeping the whole within 64 characters (FR-PRJ-08).
pub fn suffixed(name: &str, n: u32) -> String {
    if n < 2 {
        return name.to_owned();
    }
    let suffix = format!(" ({n})");
    let keep = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
    let base: String = name.chars().take(keep).collect();
    format!("{}{suffix}", base.trim_end_matches(['.', ' ']))
}

/// The first of `name`, `name (2)`, `name (3)`, … that `taken` doesn't report. Names compare
/// case-insensitively (FR-PRJ-44); `taken` gets the candidate as is.
pub fn first_free(name: &str, mut taken: impl FnMut(&str) -> bool) -> String {
    for n in 1u32.. {
        let candidate = suffixed(name, n);
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!("u32 names run out")
}

/// Case-insensitive name comparison (FR-PRJ-44).
pub fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// FR-RGN-39 slug: the prompt lower-cased, every run of characters other than letters and
/// digits → one space, trimmed, cut at a word boundary to ≤ 32 characters, then the FR-PRJ-44
/// rules; empty → `edit`.
pub fn iteration_slug(prompt: &str) -> String {
    let lowered = prompt.to_lowercase();
    let mut words = String::new();
    let mut gap = false;
    for c in lowered.chars() {
        if c.is_alphanumeric() {
            if gap && !words.is_empty() {
                words.push(' ');
            }
            gap = false;
            words.push(c);
        } else {
            gap = true;
        }
    }
    let mut slug = String::new();
    for word in words.split(' ').filter(|w| !w.is_empty()) {
        let extra = usize::from(!slug.is_empty()) + word.chars().count();
        if slug.chars().count() + extra > MAX_SLUG_CHARS {
            if slug.is_empty() {
                // One word longer than the limit: cut it.
                slug = word.chars().take(MAX_SLUG_CHARS).collect();
            }
            break;
        }
        if !slug.is_empty() {
            slug.push(' ');
        }
        slug.push_str(word);
    }
    if slug.is_empty() {
        return "edit".into();
    }
    sanitise_name(&slug)
}

/// `NNN <slug>`: three digits, more when needed (FR-RGN-39).
pub fn iteration_dir_name(n: u32, slug: &str) -> String {
    format!("{n:03} {slug}")
}

/// True if `name` is a sync client's conflict copy of `original` (FR-PRJ-34): the original's
/// stem plus `-<any>` or ` (<n>)` before the same extension, e.g. `project-OTHERPC.json` or
/// `project (2).json` for `project.json`.
pub fn is_conflict_copy(name: &str, original: &str) -> bool {
    let (stem, ext) = match original.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (original, String::new()),
    };
    let lower = name.to_lowercase();
    let Some(middle) = lower
        .strip_prefix(&stem.to_lowercase())
        .and_then(|rest| rest.strip_suffix(&ext.to_lowercase()))
    else {
        return false;
    };
    if let Some(any) = middle.strip_prefix('-') {
        return !any.is_empty();
    }
    middle
        .strip_prefix(" (")
        .and_then(|m| m.strip_suffix(')'))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prj_44_sanitises_awkward_names() {
        assert_eq!(sanitise_name("a:b?"), "a-b-");
        assert_eq!(sanitise_name("x<y>z|w*\"q\\/"), "x-y-z-w--q--");
        assert_eq!(sanitise_name("tab\there"), "tab-here");
        assert_eq!(sanitise_name("crypt. . "), "crypt");
        assert_eq!(sanitise_name("  ..hidden"), "hidden");
        assert_eq!(sanitise_name("CON"), "CON_");
        assert_eq!(sanitise_name("con"), "con_");
        assert_eq!(sanitise_name("com1"), "com1_");
        assert_eq!(sanitise_name("NUL.tar"), "NUL.tar_");
        assert_eq!(sanitise_name("CONSOLE"), "CONSOLE");
        assert_eq!(sanitise_name(""), "untitled");
        assert_eq!(sanitise_name("..."), "untitled");
        assert_eq!(sanitise_name("Grüne Höhle 洞窟"), "Grüne Höhle 洞窟");
    }

    #[test]
    fn fr_prj_44_names_are_at_most_64_chars() {
        let long = "a".repeat(80);
        assert_eq!(sanitise_name(&long).chars().count(), 64);
        let dots = format!("{}. x", "b".repeat(63));
        assert_eq!(
            sanitise_name(&dots),
            "b".repeat(63),
            "trimmed again after the cut"
        );
        assert!(is_valid_name("Crypt"));
        assert!(!is_valid_name("a:b"));
        assert!(!is_valid_name(""));
    }

    #[test]
    fn fr_prj_08_collision_suffixes() {
        assert_eq!(suffixed("map", 1), "map");
        assert_eq!(suffixed("map", 2), "map (2)");
        let long = "c".repeat(64);
        let s = suffixed(&long, 12);
        assert_eq!(s.chars().count(), 64);
        assert!(s.ends_with(" (12)"));
        let taken = ["map", "MAP (2)"];
        assert_eq!(
            first_free("Map", |c| taken.iter().any(|t| same_name(t, c))),
            "Map (3)"
        );
    }

    #[test]
    fn fr_prj_34_conflict_copy_names() {
        for yes in [
            "project-OTHERPC.json",
            "project-desktop-2.json",
            "project (2).json",
            "Project (13).JSON",
        ] {
            assert!(is_conflict_copy(yes, "project.json"), "{yes}");
        }
        for no in [
            "project.json",
            "project-.json",
            "project (x).json",
            "project (2).json.tmp",
            "projects.json",
            "other-project.json",
            ".project.json.pf-tmp-01",
        ] {
            assert!(!is_conflict_copy(no, "project.json"), "{no}");
        }
        assert!(is_conflict_copy("thumb-LAPTOP.jpg", "thumb.jpg"));
    }

    #[test]
    fn fr_rgn_39_slug_rules() {
        assert_eq!(iteration_slug("Kitchen, with   pots!"), "kitchen with pots");
        assert_eq!(iteration_slug("  "), "edit");
        assert_eq!(iteration_slug("!!!"), "edit");
        assert_eq!(
            iteration_slug("a mossy crypt floor with broken tiles and puddles"),
            "a mossy crypt floor with broken"
        );
        assert!(iteration_slug(&"x".repeat(50)).chars().count() == 32);
        assert_eq!(iteration_slug("CON"), "con_");
        assert_eq!(iteration_dir_name(3, "kitchen"), "003 kitchen");
        assert_eq!(iteration_dir_name(1234, "k"), "1234 k");
    }
}
