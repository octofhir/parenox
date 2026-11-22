//! The lexical rules for JVM symbols and keywords, before namespace resolution.

pub(super) fn valid_name(token: &str, edn: bool) -> bool {
    if valid_symbol_pattern(token) {
        return true;
    }
    // The optional leading colon may backtrack, e.g. for :2 and ::2.
    if let Some(body) = token.strip_prefix(':') {
        return valid_symbol_pattern(body) && !token[1..].contains("::");
    }
    // Clojure 1.12 added qualified primitive and reference array class names.
    !edn && token
        .rsplit_once('/')
        .is_some_and(|(namespace, dimension)| {
            !namespace.starts_with(':')
                && namespace_matches(namespace)
                && matches!(dimension.as_bytes(), [b'1'..=b'9'])
        })
}

fn valid_symbol_pattern(token: &str) -> bool {
    let body = token.strip_prefix(':').unwrap_or(token);
    if !body.bytes().any(|byte| matches!(byte, b':' | b'/')) {
        return !body.is_empty() && name_start(token);
    }
    if token.ends_with(':') || body.contains("::") {
        return false;
    }
    if token == "/" {
        return true;
    }
    if let Some(namespace) = token.strip_suffix("//") {
        return namespace_matches(namespace) && !namespace.ends_with(':');
    }
    match token.rsplit_once('/') {
        Some((namespace, name)) => {
            namespace_matches(namespace) && !namespace.ends_with(':') && name_start(name)
        }
        None => name_start(token),
    }
}

fn namespace_matches(namespace: &str) -> bool {
    name_start(namespace)
        && !namespace
            .chars()
            .skip(1)
            .any(|ch| matches!(ch, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'))
}

fn name_start(name: &str) -> bool {
    name.as_bytes()
        .first()
        .is_some_and(|b| !b.is_ascii_digit() && *b != b'/')
}
