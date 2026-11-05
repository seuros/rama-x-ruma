//! The Matrix error codes [`MatrixRejection`](crate::MatrixRejection)
//! answers with.

pub const BAD_JSON: &str = "M_BAD_JSON";
pub const EXCLUSIVE: &str = "M_EXCLUSIVE";
pub const FORBIDDEN: &str = "M_FORBIDDEN";
pub const INCOMPATIBLE_ROOM_VERSION: &str = "M_INCOMPATIBLE_ROOM_VERSION";
pub const INVALID_PARAM: &str = "M_INVALID_PARAM";
pub const INVALID_SIGNATURE: &str = "M_INVALID_SIGNATURE";
pub const INVALID_USERNAME: &str = "M_INVALID_USERNAME";
pub const LIMIT_EXCEEDED: &str = "M_LIMIT_EXCEEDED";
pub const MISSING_PARAM: &str = "M_MISSING_PARAM";
pub const MISSING_TOKEN: &str = "M_MISSING_TOKEN";
pub const NOT_FOUND: &str = "M_NOT_FOUND";
pub const NOT_YET_UPLOADED: &str = "M_NOT_YET_UPLOADED";
pub const TOO_LARGE: &str = "M_TOO_LARGE";
pub const UNAUTHORIZED: &str = "M_UNAUTHORIZED";
pub const UNABLE_TO_AUTHORISE_JOIN: &str = "M_UNABLE_TO_AUTHORISE_JOIN";
pub const UNABLE_TO_GRANT_JOIN: &str = "M_UNABLE_TO_GRANT_JOIN";
pub const UNKNOWN: &str = "M_UNKNOWN";
pub const UNKNOWN_POS: &str = "M_UNKNOWN_POS";
pub const UNKNOWN_TOKEN: &str = "M_UNKNOWN_TOKEN";
pub const UNRECOGNIZED: &str = "M_UNRECOGNIZED";
pub const UNSUPPORTED_ROOM_VERSION: &str = "M_UNSUPPORTED_ROOM_VERSION";
pub const ROOM_IN_USE: &str = "M_ROOM_IN_USE";
pub const USER_IN_USE: &str = "M_USER_IN_USE";
pub const WRONG_ROOM_KEYS_VERSION: &str = "M_WRONG_ROOM_KEYS_VERSION";
