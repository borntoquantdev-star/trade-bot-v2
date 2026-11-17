//! The student's name, kept for the submission card.
//!
//! A teacher gets a folder of screenshots and has to know whose is whose, so
//! the card the board saves carries the student's name. The name is typed into
//! the control card once and has to survive a restart, which means it lives in
//! a file: `config/student.json`, beside the `.env` the keys are kept in.
//!
//! **A name is not a secret, and that is the whole difference from the keys.**
//! `/api/control` hands it back so the field can be filled in when the page
//! opens; the keys never come back. The file is still left out of git (see
//! `.gitignore`) because it is a person's name and the repository is not the
//! place for it.
//!
//! The rules here are the same ones `app/src/submission.rs` applies to what is
//! typed, written twice because the two crates share no code. Both are small
//! and both are tested; the server's copy is the one that decides, since
//! nothing the browser says is trusted.

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::control::Refusal;

/// The file, relative to the bot folder.
pub const FILE: &str = "student.json";

/// A name longer than this is refused, not cut: cutting "สมชาย ใจดีมาก..." in
/// the middle of a surname would put a wrong name on a card with no warning.
pub const MAX_CHARS: usize = 60;

/// `<root>/config/student.json`.
pub fn path_in(root: &Path) -> PathBuf {
    root.join("config").join(FILE)
}

/// A name as it may be stored: trimmed, at most [`MAX_CHARS`] characters, and
/// free of control characters.
///
/// Control characters are refused rather than stripped. A newline in a name is
/// either a paste gone wrong or somebody trying to put a second line on a card,
/// and neither is something to quietly repair. An empty name is fine: it is how
/// a name is taken back off.
pub fn clean_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name
        .chars()
        .any(|ch| ch.is_control() || is_bidi_override(ch))
    {
        return Err("ชื่อมีอักขระควบคุม (ขึ้นบรรทัดใหม่ หรือแท็บ) · พิมพ์ชื่อบรรทัดเดียว");
    }
    if name.chars().count() > MAX_CHARS {
        return Err("ชื่อยาวเกิน 60 ตัวอักษร");
    }
    Ok(name.to_owned())
}

/// The characters that turn the text after them around (left-to-right and
/// right-to-left embeddings, overrides and isolates). They are not control
/// characters to Unicode, but a name that contains one can be made to read as
/// a different name on a card, so it is refused along with them. Thai needs
/// none of them, and the zero-width characters some Thai input leaves behind
/// are not in this list.
pub fn is_bidi_override(ch: char) -> bool {
    matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// The name in the text of a `student.json`, or an empty string if the file is
/// unreadable, is not an object, or holds a name this server would not accept.
///
/// Lenient on purpose: the file is on a student's disk and may have been edited
/// by hand. A bad file must read as "no name yet", not as an error on a page
/// that has nothing to do with it.
pub fn name_in(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.get("name").and_then(Value::as_str).map(str::to_owned))
        .and_then(|name| clean_name(&name).ok())
        .unwrap_or_default()
}

/// The stored name of the bot folder `root`, or an empty string.
pub async fn read(root: &Path) -> String {
    tokio::fs::read_to_string(path_in(root))
        .await
        .map(|text| name_in(&text))
        .unwrap_or_default()
}

/// Store `name` (already through [`clean_name`]) beside the keys.
pub fn write(root: &Path, name: &str) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(&json!({ "name": name }))
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    crate::control::write_atomically(&path_in(root), &format!("{text}\n"))
}

/// The name a request carries. A fixed sentence on every failure: serde's own
/// text can quote what was sent, and what was sent is a stranger's input.
pub fn parse_request(body: &[u8]) -> Result<String, Refusal> {
    let bad = |why: &str| Refusal::new(StatusCode::BAD_REQUEST, why.to_owned());
    let value: Value = serde_json::from_slice(body).map_err(|_| bad("อ่านข้อมูลที่ส่งมาไม่ออก"))?;
    match value.get("name") {
        Some(Value::String(name)) => clean_name(name).map_err(bad),
        _ => Err(bad("ต้องส่ง name เป็นข้อความ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_and_otherwise_left_alone() {
        assert_eq!(clean_name("  สมชาย ใจดี \n"), Ok("สมชาย ใจดี".to_owned()));
        assert_eq!(
            clean_name("Somchai   Jaidee"),
            Ok("Somchai   Jaidee".to_owned())
        );
    }

    #[test]
    fn an_empty_name_is_how_a_name_is_taken_back() {
        assert_eq!(clean_name("   "), Ok(String::new()));
    }

    #[test]
    fn a_character_that_turns_the_text_around_is_refused_but_thai_marks_are_not() {
        for bad in ["สมชาย\u{202e}ใจดี", "\u{2066}สมชาย", "a\u{202a}b"]
        {
            assert!(clean_name(bad).is_err(), "{bad:?}");
        }
        // A zero-width space is what some Thai keyboards leave between words;
        // vowels and tone marks are ordinary characters.
        assert!(clean_name("สมชาย\u{200b}ใจดี").is_ok());
        assert!(clean_name("ผู้ใหญ่ ใจดี").is_ok());
    }

    #[test]
    fn the_cap_counts_characters_not_bytes() {
        // Thai is three bytes a character; sixty of them are 180 bytes and
        // still a name that fits.
        let sixty = "ก".repeat(MAX_CHARS);
        assert_eq!(clean_name(&sixty), Ok(sixty.clone()));
        assert!(clean_name(&format!("{sixty}ก")).is_err());
    }

    #[test]
    fn a_control_character_anywhere_is_refused_not_stripped() {
        for bad in [
            "สมชาย\nใจดี",
            "สมชาย\tใจดี",
            "สมชาย\u{0}",
            "\u{7}สมชาย",
            "ก\r\nข",
        ] {
            assert!(clean_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_file_reads_back_what_was_written_and_never_fails() {
        assert_eq!(name_in(r#"{"name": "สมชาย ใจดี"}"#), "สมชาย ใจดี");
        for broken in [
            "",
            "not json",
            "[]",
            r#"{"name": 5}"#,
            r#"{"name": "a\nb"}"#,
            "{}",
        ] {
            assert_eq!(name_in(broken), "", "{broken:?}");
        }
        let too_long = format!(r#"{{"name": "{}"}}"#, "ก".repeat(MAX_CHARS + 1));
        assert_eq!(
            name_in(&too_long),
            "",
            "a hand-edited file is held to the same cap"
        );
    }

    #[test]
    fn a_request_must_say_name_and_say_it_as_text() {
        assert_eq!(
            parse_request(r#"{"name":"  สมชาย "}"#.as_bytes()).ok(),
            Some("สมชาย".to_owned())
        );
        for body in [
            &b"{}"[..],
            b"null",
            br#"{"name":3}"#,
            br#"{"name":null}"#,
            b"garbage",
        ] {
            let refused = parse_request(body).unwrap_err();
            assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn a_refusal_never_quotes_the_input() {
        let refused = parse_request(br#"{"name":"secret-ish\nvalue"}"#).unwrap_err();
        assert!(!refused.message.contains("secret"), "{}", refused.message);
    }

    #[test]
    fn it_lives_beside_the_env_file() {
        let path = path_in(Path::new("bot"));
        assert_eq!(path, Path::new("bot").join("config").join("student.json"));
    }
}
