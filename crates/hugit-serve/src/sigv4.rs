//! Minimal AWS Signature Version 4 signer for R2 (S3-compatible) GET requests.
//!
//! Hand-rolled over the workspace's existing crypto pins (`hmac` + `sha2` +
//! `hex`) — ZERO new crypto dependency, consistent with the supply-chain-strict,
//! async-free workspace. Scope is deliberately tiny: sign a single unsigned-body
//! GET (the engine only READS objects from the dedicated `<your-r2-bucket>`
//! bucket). It is NOT a general AWS SDK.
//!
//! Correctness is proven offline against the canonical AWS SigV4 test suite:
//! the `get-vanilla` request signature, the published signing-key derivation
//! vector, and the empty-payload SHA-256 — see the tests. SigV4 is auth-critical;
//! a silent signing bug would 403 every read, so it is vector-pinned, not asserted.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// SHA-256 of the empty body — the `x-amz-content-sha256` value for a bodyless GET.
pub const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// HMAC-SHA256(`key`, `msg`). `new_from_slice` is infallible for HMAC (any key
/// length is accepted), so the `expect` can never fire.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

/// Derive the SigV4 signing key: HMAC chain date → region → service → request.
fn signing_key(secret: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// RFC-3986 URI-encode `s`. Unreserved (`A-Za-z0-9-_.~`) pass through; everything
/// else is `%XX` (uppercase hex). When `keep_slash`, `/` is preserved (path use).
fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let unreserved = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && b == b'/');
        if unreserved {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// The core SigV4 authorization computation. `headers` MUST be sorted ascending
/// by lowercase name. Returns the full `Authorization` header value.
///
/// Split out (vs. the higher-level [`sign_s3_get`]) so the canonical AWS test
/// vectors can drive it directly.
#[allow(clippy::too_many_arguments)]
fn authorization(
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    headers: &[(String, String)],
    payload_hash: &str,
    amz_date: &str,
    date_stamp: &str,
    region: &str,
    service: &str,
    key_id: &str,
    secret: &str,
) -> String {
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers: String = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let signature = hex::encode(hmac_sha256(
        &signing_key(secret, date_stamp, region, service),
        string_to_sign.as_bytes(),
    ));
    format!(
        "AWS4-HMAC-SHA256 Credential={key_id}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    )
}

/// The signed headers a caller must attach to the outbound R2 GET.
#[derive(Debug, Clone)]
pub struct SignedHeaders {
    pub authorization: String,
    pub amz_date: String,
    pub content_sha256: String,
}

/// Sign a path-style R2 GET for `bucket`/`key` against `host`
/// (`<account_id>.r2.cloudflarestorage.com`). R2 uses region `auto`, service `s3`.
/// `epoch_secs` is the request time (caller passes `SystemTime::now()`-derived;
/// param for deterministic tests).
pub fn sign_s3_get(
    host: &str,
    bucket: &str,
    key: &str,
    key_id: &str,
    secret: &str,
    region: &str,
    epoch_secs: u64,
) -> SignedHeaders {
    let (amz_date, date_stamp) = format_amz_date(epoch_secs);
    // Path-style canonical URI: /<bucket>/<encoded key segments> (slash kept).
    let canonical_uri = format!("/{}/{}", uri_encode(bucket, false), uri_encode(key, true));
    // Sorted lowercase signed headers: host, x-amz-content-sha256, x-amz-date.
    let headers = vec![
        ("host".to_string(), host.to_string()),
        (
            "x-amz-content-sha256".to_string(),
            EMPTY_PAYLOAD_SHA256.to_string(),
        ),
        ("x-amz-date".to_string(), amz_date.clone()),
    ];
    let authorization = authorization(
        "GET",
        &canonical_uri,
        "",
        &headers,
        EMPTY_PAYLOAD_SHA256,
        &amz_date,
        &date_stamp,
        region,
        "s3",
        key_id,
        secret,
    );
    SignedHeaders {
        authorization,
        amz_date,
        content_sha256: EMPTY_PAYLOAD_SHA256.to_string(),
    }
}

/// URI-encode a single query-string parameter VALUE (RFC-3986, no kept slash — so a
/// prefix like `<tenant>/` encodes its `/` as `%2F`, exactly what the SigV4 canonical
/// query and the wire URL both require). Exposed so the list caller builds the ONE
/// canonical query string that is both signed and sent (they MUST match byte-for-byte).
#[must_use]
pub fn encode_query_value(v: &str) -> String {
    uri_encode(v, false)
}

/// Sign a path-style R2 **bucket LIST** (ListObjectsV2): a bodyless GET on the bucket
/// resource `/<bucket>` carrying `canonical_query` (the caller's pre-built, RFC-3986-
/// encoded, ASCII-sorted `k=v&k=v` string — e.g.
/// `continuation-token=...&list-type=2&prefix=...`). Same proven SigV4 core as
/// [`sign_s3_get`], differing only in the canonical URI (the bucket, no object key) and
/// the non-empty canonical query. The caller MUST send the IDENTICAL query on the wire.
pub fn sign_s3_list(
    host: &str,
    bucket: &str,
    canonical_query: &str,
    key_id: &str,
    secret: &str,
    region: &str,
    epoch_secs: u64,
) -> SignedHeaders {
    let (amz_date, date_stamp) = format_amz_date(epoch_secs);
    // The LIST resource is the bucket root: `/<bucket>` (no object key).
    let canonical_uri = format!("/{}", uri_encode(bucket, false));
    let headers = vec![
        ("host".to_string(), host.to_string()),
        (
            "x-amz-content-sha256".to_string(),
            EMPTY_PAYLOAD_SHA256.to_string(),
        ),
        ("x-amz-date".to_string(), amz_date.clone()),
    ];
    let authorization = authorization(
        "GET",
        &canonical_uri,
        canonical_query,
        &headers,
        EMPTY_PAYLOAD_SHA256,
        &amz_date,
        &date_stamp,
        region,
        "s3",
        key_id,
        secret,
    );
    SignedHeaders {
        authorization,
        amz_date,
        content_sha256: EMPTY_PAYLOAD_SHA256.to_string(),
    }
}

/// Sign a path-style R2 PUT of `body` to `bucket`/`key`. Identical SigV4 algorithm
/// to [`sign_s3_get`] (same proven [`authorization`] core), differing only in the
/// method (`PUT`) and — the one PUT-specific bit — the `x-amz-content-sha256` /
/// payload hash, which is the SHA-256 of the ACTUAL body (a bodyless-GET's empty
/// hash would make R2 reject a non-empty PUT). Used by the one-shot snapshot
/// uploader (the `hugit-snapshot` bin), which requires a read+WRITE credential —
/// the standing engine cred is read-only by design.
#[allow(clippy::too_many_arguments)]
pub fn sign_s3_put(
    host: &str,
    bucket: &str,
    key: &str,
    body: &[u8],
    key_id: &str,
    secret: &str,
    region: &str,
    epoch_secs: u64,
) -> SignedHeaders {
    let (amz_date, date_stamp) = format_amz_date(epoch_secs);
    let payload_hash = sha256_hex(body);
    let canonical_uri = format!("/{}/{}", uri_encode(bucket, false), uri_encode(key, true));
    let headers = vec![
        ("host".to_string(), host.to_string()),
        ("x-amz-content-sha256".to_string(), payload_hash.clone()),
        ("x-amz-date".to_string(), amz_date.clone()),
    ];
    let authorization = authorization(
        "PUT",
        &canonical_uri,
        "",
        &headers,
        &payload_hash,
        &amz_date,
        &date_stamp,
        region,
        "s3",
        key_id,
        secret,
    );
    SignedHeaders {
        authorization,
        amz_date,
        content_sha256: payload_hash,
    }
}

/// Format a Unix epoch-seconds instant as (`YYYYMMDDTHHMMSSZ`, `YYYYMMDD`) UTC.
fn format_amz_date(epoch_secs: u64) -> (String, String) {
    let days = (epoch_secs / 86_400) as i64;
    let sod = epoch_secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s) = (sod / 3600, (sod % 3600) / 60, sod % 60);
    (
        format!("{y:04}{m:02}{d:02}T{h:02}{mi:02}{s:02}Z"),
        format!("{y:04}{m:02}{d:02}"),
    )
}

/// Days since the Unix epoch → (year, month, day), proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS test suite `get-vanilla`: the canonical GET-/ signature is published.
    /// This pins the WHOLE algorithm (canonical request · string-to-sign ·
    /// signing-key chain · final HMAC) to a known-good output.
    #[test]
    fn aws_sigv4_get_vanilla_vector() {
        let headers = vec![
            ("host".to_string(), "example.amazonaws.com".to_string()),
            ("x-amz-date".to_string(), "20150830T123600Z".to_string()),
        ];
        let auth = authorization(
            "GET",
            "/",
            "",
            &headers,
            EMPTY_PAYLOAD_SHA256,
            "20150830T123600Z",
            "20150830",
            "us-east-1",
            "service",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        );
        assert!(
            auth.ends_with(
                "Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
            ),
            "get-vanilla signature mismatch: {auth}"
        );
        assert!(auth.contains("SignedHeaders=host;x-amz-date"));
        assert!(auth.contains("Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request"));
    }

    /// AWS docs published signing-key derivation: secret + 20150830/us-east-1/iam
    /// → a known hex key. Pins the HMAC chain independently of the request.
    #[test]
    fn aws_signing_key_derivation_vector() {
        let k = signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20120215",
            "us-east-1",
            "iam",
        );
        assert_eq!(
            hex::encode(k),
            "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d"
        );
    }

    #[test]
    fn empty_payload_hash_is_the_known_constant() {
        assert_eq!(sha256_hex(b""), EMPTY_PAYLOAD_SHA256);
    }

    /// RFC 4231 test case 2 — pins the HMAC-SHA256 primitive to a certain vector,
    /// independent of the SigV4 structure above.
    #[test]
    fn rfc4231_hmac_sha256_case2() {
        assert_eq!(
            hex::encode(hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn amz_date_formats_utc() {
        assert_eq!(
            format_amz_date(0),
            ("19700101T000000Z".to_string(), "19700101".to_string())
        );
        assert_eq!(
            format_amz_date(86_400 + 45_296),
            ("19700102T123456Z".to_string(), "19700102".to_string())
        );
    }

    #[test]
    fn uri_encode_keeps_slash_in_path_only() {
        assert_eq!(
            uri_encode("tenant-uuid/my-repo.json", true),
            "tenant-uuid/my-repo.json"
        );
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
        assert_eq!(uri_encode("a b+c", true), "a%20b%2Bc");
    }

    /// The real R2 GET signer wires host/bucket/key + the S3 headers; smoke-check
    /// it produces a well-formed Authorization with the s3/auto scope.
    #[test]
    fn sign_s3_get_is_well_formed() {
        let s = sign_s3_get(
            "acct.r2.cloudflarestorage.com",
            "example-bucket",
            "tenant-uuid/hugit.json",
            "AKIDEXAMPLE",
            "secret",
            "auto",
            1_700_000_000,
        );
        assert!(
            s.authorization
                .starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/")
        );
        assert!(s.authorization.contains("/auto/s3/aws4_request"));
        assert!(
            s.authorization
                .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date")
        );
        assert_eq!(s.content_sha256, EMPTY_PAYLOAD_SHA256);
        assert!(s.amz_date.ends_with('Z'));
    }

    /// The PUT signer must hash the REAL body into `x-amz-content-sha256` (a
    /// bodyless-GET's empty-hash would make R2 reject the PUT) and otherwise share
    /// the proven authorization core. Pins the body-hash to the known SHA-256 of
    /// `[]` and asserts the PUT scope + that the content hash is NOT the empty one.
    #[test]
    fn sign_s3_put_hashes_the_real_body() {
        let body = b"[]"; // a minimal (empty) event log
        let s = sign_s3_put(
            "acct.r2.cloudflarestorage.com",
            "example-bucket",
            "tenant-uuid/hugit.json",
            body,
            "AKIDEXAMPLE",
            "secret",
            "auto",
            1_700_000_000,
        );
        // SHA-256("[]") — verified via `shasum -a 256` + Python hashlib (NOT recalled).
        assert_eq!(
            s.content_sha256,
            "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".to_string(),
            "PUT must carry the SHA-256 of the actual body"
        );
        assert_ne!(
            s.content_sha256, EMPTY_PAYLOAD_SHA256,
            "PUT must NOT carry the bodyless-GET empty-payload hash"
        );
        assert!(
            s.authorization
                .starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/")
        );
        assert!(s.authorization.contains("/auto/s3/aws4_request"));
        assert!(
            s.authorization
                .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date")
        );
    }
}
