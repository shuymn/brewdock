use std::{cmp::Ordering, fmt, str::FromStr};

/// A parsed package version that supports Homebrew-compatible comparison.
///
/// Version strings are tokenized into typed segments and compared
/// token-by-token following the same algorithm as Homebrew's `Version`
/// class.  This correctly handles pre-release suffixes (`alpha`, `beta`,
/// `pre`, `rc`), post-release indicators (`p`, `.post`), pure-numeric
/// versions (including date-style `20240101`), and revision suffixes
/// (`_1`).
///
/// # Examples
///
/// ```
/// use brewdock_formula::PkgVersion;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let a: PkgVersion = "3.51.3".parse()?;
/// let b: PkgVersion = "3.52.0".parse()?;
/// assert!(a < b);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct PkgVersion {
    tokens: Vec<Token>,
    /// Preserved for lossless [`Display`] — separator information is
    /// lost during tokenization, so the original string is kept.
    raw: String,
}

/// A single token extracted from a version string.
///
/// The discriminant order encodes the Homebrew release-stage hierarchy
/// used when comparing tokens of different kinds:
///
/// `Alpha < Beta < Pre < Rc < Null ≈ Numeric(0) < Patch < Post`
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// Pre-release: `alpha`, `alpha1`, `a2`, …
    Alpha(u64),
    /// Pre-release: `beta`, `beta1`, `b2`, …
    Beta(u64),
    /// Pre-release: `pre`, `pre1`, …
    Pre(u64),
    /// Release candidate: `rc`, `rc1`, …
    Rc(u64),
    /// Pure numeric segment: `0`, `42`, `20240101`, …
    Numeric(u64),
    /// Post-stable patch: `p`, `p1`, …
    Patch(u64),
    /// Post-stable: `.post1`, …
    Post(u64),
    /// Unrecognised alphabetic segment.
    Text(String),
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Errors that can occur when parsing a version string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseVersionError {
    input: String,
}

impl fmt::Display for ParseVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid version string: {}", self.input)
    }
}

impl std::error::Error for ParseVersionError {}

impl FromStr for PkgVersion {
    type Err = ParseVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let tokens = tokenize(s);
        if tokens.is_empty() {
            return Err(ParseVersionError {
                input: s.to_owned(),
            });
        }
        Ok(Self {
            tokens,
            raw: s.to_owned(),
        })
    }
}

/// Scans `input` and returns a list of [`Token`]s.
///
/// Separator characters (`.`, `-`, `_`) are consumed but not emitted.
/// The scanner tries composite-keyword patterns (`alpha`, `beta`, `pre`,
/// `rc`, `post`, `p`) before falling back to pure-numeric or
/// pure-alphabetic segments.
fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        let byte = bytes[pos];

        if byte == b'.' || byte == b'-' || byte == b'_' {
            // `.post` must keep the dot as part of the pattern so that
            // it is not confused with `p` + `ost`.
            if byte == b'.'
                && pos + 4 < len
                && bytes[pos + 1..].starts_with(b"post")
                && (pos + 5 >= len || !bytes[pos + 5].is_ascii_alphabetic())
            {
                let start = pos + 5;
                let end = advance_digits(bytes, start);
                let rev = parse_suffix(bytes, start, end);
                tokens.push(Token::Post(rev));
                pos = end;
                continue;
            }
            pos += 1;
            continue;
        }

        if byte.is_ascii_digit() {
            let end = advance_digits(bytes, pos);
            let value = digits_to_u64(bytes, pos, end);
            tokens.push(Token::Numeric(value));
            pos = end;
            continue;
        }

        if byte.is_ascii_alphabetic() {
            if let Some((tok, end)) = try_keyword(bytes, pos) {
                tokens.push(tok);
                pos = end;
                continue;
            }

            let end = advance_alpha(bytes, pos);
            let word = std::str::from_utf8(&bytes[pos..end])
                .unwrap_or_default()
                .to_ascii_lowercase();
            tokens.push(Token::Text(word));
            pos = end;
            continue;
        }

        pos += 1;
    }

    tokens
}

type KeywordEntry = (&'static [u8], fn(u64) -> Token);
type ShortKeywordEntry = (u8, fn(u64) -> Token);

/// Long-form keyword table: `(prefix_bytes, constructor)`.
///
/// Ordered longest-first to avoid ambiguous prefix matches.
const LONG_KEYWORDS: &[KeywordEntry] = &[
    (b"alpha", Token::Alpha as fn(u64) -> Token),
    (b"beta", Token::Beta),
    (b"pre", Token::Pre),
    (b"rc", Token::Rc),
];

/// Short-form keyword table: `(single_char, constructor)`.
///
/// `a[0-9]+` → Alpha, `b[0-9]+` → Beta.  Requires at least one
/// trailing digit to avoid matching plain words.
const SHORT_KEYWORDS: &[ShortKeywordEntry] = &[
    (b'a', Token::Alpha as fn(u64) -> Token),
    (b'b', Token::Beta),
];

/// Attempts to match a keyword prefix at `pos` in `bytes`.
///
/// Keyword patterns (case-insensitive):
///   `alpha[0-9]*` | `a[0-9]+`  →  Alpha
///   `beta[0-9]*`  | `b[0-9]+`  →  Beta
///   `pre[0-9]*`                 →  Pre
///   `rc[0-9]*`                  →  Rc
///   `p[0-9]+`                   →  Patch (requires at least one digit)
///   `p` (end-of-segment)        →  Patch(0)
///
/// Returns `Some((token, end_index))` on match, `None` otherwise.
fn try_keyword(bytes: &[u8], pos: usize) -> Option<(Token, usize)> {
    let remaining = &bytes[pos..];

    for &(prefix, ctor) in LONG_KEYWORDS {
        if let Some(rest) = strip_prefix_ci(remaining, prefix)
            && (rest.is_empty() || !rest[0].is_ascii_alphabetic())
        {
            let start = pos + prefix.len();
            let end = advance_digits(bytes, start);
            let rev = parse_suffix(bytes, start, end);
            return Some((ctor(rev), end));
        }
    }

    // Short-form: single letter followed by digits, not inside a word.
    for &(letter, ctor) in SHORT_KEYWORDS {
        if remaining.len() >= 2
            && remaining[0].eq_ignore_ascii_case(&letter)
            && remaining[1].is_ascii_digit()
            && (pos == 0 || !bytes[pos - 1].is_ascii_alphabetic())
        {
            let start = pos + 1;
            let end = advance_digits(bytes, start);
            if end > start {
                let rev = digits_to_u64(bytes, start, end);
                return Some((ctor(rev), end));
            }
        }
    }

    // `p[0-9]*` → Patch (not inside a word).
    if remaining[0].eq_ignore_ascii_case(&b'p')
        && (pos == 0 || !bytes[pos - 1].is_ascii_alphabetic())
    {
        let start = pos + 1;
        let end = advance_digits(bytes, start);
        if end > start {
            let rev = digits_to_u64(bytes, start, end);
            return Some((Token::Patch(rev), end));
        }
        if start >= bytes.len() || is_separator(bytes[start]) {
            return Some((Token::Patch(0), start));
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// Rank used for cross-type ordering.
///
/// Matches the Homebrew hierarchy:
/// `Alpha < Beta < Pre < Rc < Null ≈ Numeric(0) < Patch < Post`
const fn stage_rank(token: &Token) -> u8 {
    match token {
        Token::Alpha(_) => 0,
        Token::Beta(_) => 1,
        Token::Pre(_) => 2,
        Token::Rc(_) => 3,
        Token::Numeric(_) => 4,
        Token::Patch(_) => 5,
        Token::Post(_) => 6,
        Token::Text(_) => 7,
    }
}

/// Returns the numeric revision embedded in a composite token, or the
/// value itself for [`Token::Numeric`].
const fn token_rev(token: &Token) -> u64 {
    match token {
        Token::Alpha(rev)
        | Token::Beta(rev)
        | Token::Pre(rev)
        | Token::Rc(rev)
        | Token::Numeric(rev)
        | Token::Patch(rev)
        | Token::Post(rev) => *rev,
        Token::Text(_) => 0,
    }
}

/// Compares two tokens following Homebrew semantics.
fn cmp_tokens(left: &Token, right: &Token) -> Ordering {
    if stage_rank(left) == stage_rank(right) {
        match (left, right) {
            (Token::Text(tl), Token::Text(tr)) => tl.cmp(tr),
            _ => token_rev(left).cmp(&token_rev(right)),
        }
    } else {
        stage_rank(left).cmp(&stage_rank(right))
    }
}

/// Sentinel representing a missing token when one version is shorter.
///
/// Homebrew rules:
/// - `Null == Numeric(0)`
/// - `Null > Alpha | Beta | Pre | Rc`
/// - `Null < Numeric(>0) | Patch | Post`
const NULL_RANK: u8 = 4; // same as Numeric

/// Compares a real token against the Null sentinel.
fn cmp_token_vs_null(token: &Token) -> Ordering {
    match token {
        Token::Numeric(0) => Ordering::Equal,
        Token::Numeric(_) | Token::Patch(_) | Token::Post(_) => Ordering::Greater,
        Token::Alpha(_) | Token::Beta(_) | Token::Pre(_) | Token::Rc(_) => Ordering::Less,
        Token::Text(_) => stage_rank(token).cmp(&NULL_RANK),
    }
}

impl Ord for PkgVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.raw == other.raw {
            return Ordering::Equal;
        }

        let left_tokens = &self.tokens;
        let right_tokens = &other.tokens;
        let max = left_tokens.len().max(right_tokens.len());
        let mut li = 0usize;
        let mut ri = 0usize;

        while li < max || ri < max {
            let left = left_tokens.get(li);
            let right = right_tokens.get(ri);

            match (left, right) {
                (None, None) => break,
                (Some(lt), None) => {
                    let ord = cmp_token_vs_null(lt);
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    li += 1;
                }
                (None, Some(rt)) => {
                    let ord = cmp_token_vs_null(rt);
                    if ord != Ordering::Equal {
                        return ord.reverse();
                    }
                    ri += 1;
                }
                (Some(lt), Some(rt)) => {
                    let left_numeric = matches!(lt, Token::Numeric(_));
                    let right_numeric = matches!(rt, Token::Numeric(_));

                    if left_numeric == right_numeric {
                        let ord = cmp_tokens(lt, rt);
                        if ord != Ordering::Equal {
                            return ord;
                        }
                        li += 1;
                        ri += 1;
                    } else if left_numeric {
                        if cmp_token_vs_null(lt) == Ordering::Greater {
                            return Ordering::Greater;
                        }
                        li += 1;
                    } else {
                        if cmp_token_vs_null(rt) == Ordering::Greater {
                            return Ordering::Less;
                        }
                        ri += 1;
                    }
                }
            }
        }

        Ordering::Equal
    }
}

impl PartialOrd for PkgVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for PkgVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for PkgVersion {}

impl fmt::Display for PkgVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn advance_digits(bytes: &[u8], start: usize) -> usize {
    let mut idx = start;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    idx
}

fn advance_alpha(bytes: &[u8], start: usize) -> usize {
    let mut idx = start;
    while idx < bytes.len() && bytes[idx].is_ascii_alphabetic() {
        idx += 1;
    }
    idx
}

/// Converts a slice of ASCII digit bytes to `u64`, saturating on
/// overflow instead of silently returning 0.
fn digits_to_u64(bytes: &[u8], start: usize, end: usize) -> u64 {
    bytes[start..end].iter().fold(0u64, |acc, &digit| {
        acc.saturating_mul(10)
            .saturating_add(u64::from(digit - b'0'))
    })
}

fn parse_suffix(bytes: &[u8], start: usize, end: usize) -> u64 {
    if end > start {
        digits_to_u64(bytes, start, end)
    } else {
        0
    }
}

const fn is_separator(byte: u8) -> bool {
    byte == b'.' || byte == b'-' || byte == b'_'
}

/// Case-insensitive prefix match. Returns the remainder on success.
fn strip_prefix_ci<'a>(haystack: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    if haystack.len() < needle.len() {
        return None;
    }
    for (hay, ndl) in haystack.iter().zip(needle.iter()) {
        if !hay.eq_ignore_ascii_case(ndl) {
            return None;
        }
    }
    Some(&haystack[needle.len()..])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> PkgVersion {
        s.parse()
            .unwrap_or_else(|_| unreachable!("test version string must be valid: {s:?}"))
    }

    fn assert_order(lesser: &str, greater: &str) {
        let a = v(lesser);
        let b = v(greater);
        assert!(a < b, "expected {lesser} < {greater}, got {:?}", a.cmp(&b));
        assert!(b > a, "expected {greater} > {lesser}, got {:?}", b.cmp(&a));
    }

    fn assert_equal(left: &str, right: &str) {
        let a = v(left);
        let b = v(right);
        assert!(a == b, "expected {left} == {right}, got {:?}", a.cmp(&b));
    }

    // -- Basic numeric -----------------------------------------------------

    #[test]
    fn test_simple_numeric_ordering() {
        assert_order("1.0", "2.0");
        assert_order("1.0", "1.1");
        assert_order("1.9", "1.10");
        assert_order("0.1", "0.2");
    }

    #[test]
    fn test_multi_segment_numeric() {
        assert_order("1.0.0", "1.0.1");
        assert_order("1.0.9", "1.1.0");
        assert_order("3.51.3", "3.52.0");
    }

    #[test]
    fn test_sqlite_bug_case() {
        // The original bug: installed 3.52.0 was reported as outdated
        // because cached latest was 3.51.3.
        assert_order("3.51.3", "3.52.0");
    }

    // -- Null padding (different lengths) ----------------------------------

    #[test]
    fn test_trailing_zero_equivalence() {
        assert_equal("1.0", "1.0.0");
        assert_equal("1.0.0", "1.0.0.0");
    }

    #[test]
    fn test_shorter_less_than_longer_nonzero() {
        assert_order("1.0", "1.0.1");
        assert_order("1.0.0", "1.0.0.1");
    }

    // -- Revision suffix ---------------------------------------------------

    #[test]
    fn test_revision_suffix() {
        assert_order("3.52.0", "3.52.0_1");
        assert_order("3.52.0_1", "3.52.0_2");
    }

    // -- Pre-release ordering ----------------------------------------------

    #[test]
    fn test_prerelease_less_than_stable() {
        assert_order("1.0alpha1", "1.0");
        assert_order("1.0beta1", "1.0");
        assert_order("1.0pre1", "1.0");
        assert_order("1.0rc1", "1.0");
    }

    #[test]
    fn test_prerelease_hierarchy() {
        assert_order("1.0alpha1", "1.0beta1");
        assert_order("1.0beta1", "1.0pre1");
        assert_order("1.0pre1", "1.0rc1");
        assert_order("1.0rc1", "1.0");
    }

    #[test]
    fn test_prerelease_revisions() {
        assert_order("1.0alpha1", "1.0alpha2");
        assert_order("1.0beta1", "1.0beta2");
        assert_order("1.0rc1", "1.0rc2");
    }

    #[test]
    fn test_short_prerelease_forms() {
        assert_order("1.0a1", "1.0a2");
        assert_order("1.0b1", "1.0b2");
        assert_order("1.0a1", "1.0b1");
    }

    // -- Post-release / patch ----------------------------------------------

    #[test]
    fn test_patch_greater_than_stable() {
        assert_order("1.0", "1.0p1");
        assert_order("1.0p1", "1.0p2");
    }

    // -- Date-style versions -----------------------------------------------

    #[test]
    fn test_date_versions() {
        assert_order("20240101", "20240202");
        assert_order("20231231", "20240101");
    }

    // -- Dotted pre-release ------------------------------------------------

    #[test]
    fn test_dotted_prerelease() {
        assert_order("1.0.0.alpha1", "1.0.0.beta1");
        assert_order("1.0.0.rc1", "1.0.0");
    }

    // -- Display -----------------------------------------------------------

    #[test]
    fn test_display_preserves_raw() {
        let ver = v("3.52.0_1");
        assert_eq!(ver.to_string(), "3.52.0_1");
    }

    // -- Parse errors ------------------------------------------------------

    #[test]
    fn test_empty_string_is_error() {
        assert!("".parse::<PkgVersion>().is_err());
    }

    // -- Eq / identity -----------------------------------------------------

    #[test]
    fn test_identical_strings_are_equal() {
        assert_equal("1.2.3", "1.2.3");
        assert_equal("3.52.0_1", "3.52.0_1");
    }

    // -- Case insensitivity ------------------------------------------------

    #[test]
    fn test_case_insensitive_keywords() {
        assert_equal("1.0Alpha1", "1.0alpha1");
        assert_equal("1.0RC1", "1.0rc1");
    }

    // -- Post token --------------------------------------------------------

    #[test]
    fn test_post_token() {
        assert_order("1.0", "1.0.post1");
        assert_order("1.0p1", "1.0.post1");
    }
}
