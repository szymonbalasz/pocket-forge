//! Credentials in memory (FR-PRV-06/07/08). Neither the key nor the secret can be printed,
//! logged or serialised: [`Secret`] redacts `Debug`/`Display` and has no `Serialize`.

use std::fmt;

/// A value that must never reach logs, errors, files or the UI.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    /// The raw value. Only the transport (auth header) and the credential store call this.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// A Scenario API key pair. Both halves are secret; only the last 4 characters of the key id
/// are ever shown (FR-PRV-07).
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub key: Secret,
    pub secret: Secret,
}

impl Credentials {
    pub fn new(key: impl Into<String>, secret: impl Into<String>) -> Self {
        Credentials {
            key: Secret::new(key),
            secret: Secret::new(secret),
        }
    }

    /// The last 4 characters of the key id (FR-PRV-07).
    pub fn key_suffix(&self) -> String {
        let chars: Vec<char> = self.key.expose().chars().collect();
        chars[chars.len().saturating_sub(4)..].iter().collect()
    }

    /// The HTTP Basic header value, `Basic base64(key:secret)`. Itself a [`Secret`].
    pub fn basic_auth(&self) -> Secret {
        use base64::Engine as _;
        let raw = format!("{}:{}", self.key.expose(), self.secret.expose());
        Secret::new(format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        ))
    }

    /// Removes the key, the secret and the auth header value from text that came from outside
    /// (provider error hints), so an echoing server can't leak them into errors (FR-PRV-23).
    pub fn scrub(&self, text: &str) -> String {
        let mut out = text.to_owned();
        let auth = self.basic_auth();
        let token = auth.expose().trim_start_matches("Basic ").to_owned();
        for needle in [
            auth.expose(),
            &token,
            self.secret.expose(),
            self.key.expose(),
        ] {
            if !needle.is_empty() {
                out = out.replace(needle, "<redacted>");
            }
        }
        out
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("key", &format_args!("<redacted …{}>", self.key_suffix()))
            .field("secret", &self.secret)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check: this fails to compile (ambiguous impl) if `Secret: Serialize`.
    trait NotSerialize<A> {
        fn check() {}
    }
    impl<T: ?Sized> NotSerialize<()> for T {}
    struct IsSerialize;
    impl<T: ?Sized + serde::Serialize> NotSerialize<IsSerialize> for T {}

    #[test]
    fn fr_prv_08_secret_redacts_debug_and_display_and_has_no_serialize() {
        <Secret as NotSerialize<_>>::check();
        <Credentials as NotSerialize<_>>::check();
        let c = Credentials::new("key-id-0000abcd", "very-secret-value");
        let shown = format!("{c:?} {} {:?} {}", c.secret, c.secret, c.basic_auth());
        assert!(!shown.contains("very-secret-value"));
        assert!(!shown.contains("key-id-0000"));
        assert!(!shown.contains("Basic"));
        assert!(shown.contains("abcd"), "the key suffix is shown: {shown}");
    }

    #[test]
    fn fr_prv_07_key_suffix_is_the_last_four_characters() {
        assert_eq!(Credentials::new("abcdef1234", "s").key_suffix(), "1234");
        assert_eq!(Credentials::new("ab", "s").key_suffix(), "ab");
    }

    #[test]
    fn fr_prv_23_scrub_removes_credentials_from_outside_text() {
        let c = Credentials::new("kkkk", "ssss");
        let auth = c.basic_auth();
        let text = format!("bad header {} for kkkk with ssss", auth.expose());
        let clean = c.scrub(&text);
        assert!(!clean.contains("ssss") && !clean.contains("kkkk") && !clean.contains("Basic a"));
    }
}
