//! Rules for the names a user gives a meeting and its speakers. A speaker
//! is renamed once, by key, and every bubble and export shows the new name,
//! so no stored text is ever rewritten.

pub const MAX_SPEAKER_CHARS: usize = 60;
pub const MAX_TITLE_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
}

impl std::fmt::Display for NameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "The name is empty",
            Self::TooLong => "The name is too long",
        })
    }
}

/// Runs of whitespace, line breaks included, become one space.
fn collapse(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn speaker_name(input: &str) -> Result<String, NameError> {
    let name = collapse(input);
    if name.is_empty() {
        Err(NameError::Empty)
    } else if name.chars().count() > MAX_SPEAKER_CHARS {
        Err(NameError::TooLong)
    } else {
        Ok(name)
    }
}

/// `None` clears the title, so the meeting shows its default name again.
pub fn title(input: &str) -> Result<Option<String>, NameError> {
    let title = collapse(input);
    if title.chars().count() > MAX_TITLE_CHARS {
        Err(NameError::TooLong)
    } else {
        Ok((!title.is_empty()).then_some(title))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_names_are_trimmed_and_bounded() {
        assert_eq!(speaker_name("  Alex \n Martin "), Ok("Alex Martin".into()));
        assert_eq!(speaker_name(" \t "), Err(NameError::Empty));
        assert_eq!(speaker_name(&"é".repeat(60)), Ok("é".repeat(60)));
        assert_eq!(speaker_name(&"é".repeat(61)), Err(NameError::TooLong));
    }

    #[test]
    fn an_empty_title_restores_the_default() {
        assert_eq!(title("  Point hebdo  "), Ok(Some("Point hebdo".into())));
        assert_eq!(title("   "), Ok(None));
        assert_eq!(title(&"a".repeat(121)), Err(NameError::TooLong));
    }
}
