//! Percent-decoding of query values.

use std::borrow::Cow;
use std::str::Utf8Error;

use rama::net::uri::util::percent_encoding::percent_decode_str;

/// Decode `%XX` escapes in `data`.
///
/// `+` stays `+` (not a space) and malformed escapes such as `%zz` pass
/// through literally. Fails only when the decoded bytes are not UTF-8.
pub(crate) fn decode(data: &str) -> Result<Cow<'_, str>, Utf8Error> {
    percent_decode_str(data).decode_utf8()
}
