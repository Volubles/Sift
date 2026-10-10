use crate::{Error, Result};

pub(crate) mod selection;

#[derive(Debug)]
pub enum SelectionSources {
    Ready(Vec<std::path::PathBuf>),
    MissingEndpoint,
    TooLarge,
}

pub const PAGE_SIZE: usize = 128;
const MAX_QUERY_CHARS: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SearchQuery {
    #[default]
    All,
    FilenamePrefix {
        lower: String,
        upper: String,
    },
    Substrings(String),
}

impl SearchQuery {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Ok(Self::All);
        }
        if input.chars().count() > MAX_QUERY_CHARS {
            return Err(Error::InvalidQuery(
                "Search is limited to 256 characters".into(),
            ));
        }
        let terms: Vec<_> = input.split_whitespace().collect();
        if terms.len() == 1 && input.chars().count() < 3 {
            let lower = input.to_lowercase();
            let upper = prefix_successor(&lower)
                .ok_or_else(|| Error::InvalidQuery("This prefix cannot be indexed".into()))?;
            return Ok(Self::FilenamePrefix { lower, upper });
        }
        if terms.iter().any(|term| term.chars().count() < 3) {
            return Err(Error::InvalidQuery(
                "Use at least 3 characters per word, or a short filename prefix".into(),
            ));
        }
        // Quote each literal term: user punctuation must never become FTS syntax.
        let expression = terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        Ok(Self::Substrings(expression))
    }
}

fn prefix_successor(prefix: &str) -> Option<String> {
    let mut chars: Vec<_> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let mut next = u32::from(last) + 1;
        if (0xd800..=0xdfff).contains(&next) {
            next = 0xe000;
        }
        if let Some(successor) = char::from_u32(next) {
            chars.push(successor);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/unit/search.rs"));
}
