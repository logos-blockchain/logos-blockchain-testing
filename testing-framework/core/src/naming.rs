/// Returns whether `name` is a lowercase DNS label within `max_len` bytes.
#[must_use]
pub fn is_valid_dns_label(name: &str, max_len: usize) -> bool {
    !name.is_empty()
        && name.len() <= max_len
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

/// Returns whether a cluster name is portable across deployment backends.
#[must_use]
pub fn is_valid_cluster_name(name: &str) -> bool {
    const MAX_CLUSTER_NAME_LEN: usize = 32;
    is_valid_dns_label(name, MAX_CLUSTER_NAME_LEN)
}

#[cfg(test)]
mod tests {
    use super::{is_valid_cluster_name, is_valid_dns_label};

    #[test]
    fn dns_labels_respect_syntax_and_length() {
        assert!(is_valid_dns_label("alpha-2", 63));
        assert!(!is_valid_dns_label("Bad_Name", 63));
        assert!(!is_valid_dns_label("-alpha", 63));
        assert!(!is_valid_dns_label("alpha-", 63));
        assert!(!is_valid_dns_label("alpha", 4));
    }

    #[test]
    fn portable_cluster_names_are_limited_to_32_bytes() {
        assert!(is_valid_cluster_name("alpha-2"));
        assert!(!is_valid_cluster_name(&"a".repeat(33)));
    }
}
