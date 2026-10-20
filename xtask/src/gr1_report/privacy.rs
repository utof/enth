//! The privacy lint. A round log must hold no personal data: no names, no email addresses, no paths with a user
//! name in them. The plugin writes none of these (the log has codes, patches, scores and times), but a dispute note is
//! free text and a patch has a name field, so the report looks for the two obvious leaks before the owner shares the
//! numbers or commits a log. It reports **where and what kind, never the text itself**.

/// Findings in one raw line of a log: `"an email address"` or `"a home-folder path"`, each at most once.
pub fn scan(line: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    if has_email(line) {
        found.push("an email address");
    }
    if has_home_path(line) {
        found.push("a home-folder path");
    }
    found
}

fn is_local(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')
}

fn is_domain(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-')
}

/// `local@domain.tld`: something before the `@`, and after it a dotted name ending in at least two letters.
fn has_email(line: &str) -> bool {
    for (at, _) in line.match_indices('@') {
        let before = line[..at].chars().next_back();
        if !before.is_some_and(is_local) {
            continue;
        }
        let domain: String = line[at + 1..]
            .chars()
            .take_while(|c| is_domain(*c))
            .collect();
        let domain = domain.trim_end_matches('.');
        if let Some((name, tld)) = domain.rsplit_once('.')
            && !name.is_empty()
            && tld.len() >= 2
            && tld.chars().all(|c| c.is_ascii_alphabetic())
        {
            return true;
        }
    }
    false
}

/// `/home/<name>`, `/Users/<name>`, `C:\Users\<name>` (also with the backslashes JSON-escaped): a user folder with
/// a name after it.
fn has_home_path(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ["/home/", "/users/", "\\users\\", "\\\\users\\\\"]
        .iter()
        .any(|marker| {
            lower.match_indices(marker).any(|(at, _)| {
                lower[at + marker.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_found_and_codes_and_patches_are_not() {
        assert_eq!(
            scan(r#"{"dispute":"mail me at someone@example.com please"}"#),
            vec!["an email address"]
        );
        assert_eq!(scan("a.b+c@sub.example.co.uk"), vec!["an email address"]);
        let none: [&str; 0] = [];
        assert_eq!(
            scan(r#"{"code":"1080-1000-0001-AB","patch":"{\"schema\":1,\"engine\":\"0.2.0\"}"}"#),
            none
        );
        assert_eq!(
            scan("the cutoff @ 2 kHz"),
            none,
            "an at sign alone is not an address"
        );
        assert_eq!(scan("write @home"), none);
        assert_eq!(scan("a@b"), none, "no dotted domain");
    }

    #[test]
    fn user_folders_are_found_in_every_spelling() {
        for text in [
            "/home/alice/Documents/enth",
            "/Users/alice/Library",
            r"C:\Users\alice\Documents",
            r"C:\\Users\\alice\\Documents",
            "c:/users/alice",
        ] {
            assert_eq!(scan(text), vec!["a home-folder path"], "{text}");
        }
        assert!(
            scan("/home/").is_empty()
                && scan("/usr/home").is_empty()
                && scan("Users of the synth").is_empty()
        );
    }

    #[test]
    fn findings_never_echo_the_text() {
        let found = scan("alice@example.com in /home/alice");
        assert_eq!(found, vec!["an email address", "a home-folder path"]);
        assert!(found.iter().all(|f| !f.contains("alice")));
    }
}
