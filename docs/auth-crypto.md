# Credential and cryptography contracts (stage 04)

`daraja-core::auth::{CollectionApp, DisbursementApp}` are distinct typed OAuth actors.
Each constructor creates its own credentials, HTTP client, cache, refresh mutex,
background timer and cancellation token. Clones share only that actor. Operators
must configure distinct provider apps with the appropriate permissions; types do
not establish provider permissions or prevent an operator supplying the same keys.
Stage 05 collection/status transports consume these contracts; stage 06 adds payout transports. See docs/collection-transport.md for routing and the no-replay 401 boundary.

Construct inside a Tokio runtime. OAuth endpoints must use HTTPS, except numeric
loopback HTTP for local fixtures. User info, query strings and fragments are
rejected; the actor adds `grant_type=client_credentials` and HTTP Basic auth.
Redirects and automatic HTTP retries are disabled. Requests and full response
consumption have a 15-second deadline; successful bodies are limited to 16 KiB.
Errors never carry provider bodies, URLs, credentials or nested transport errors.

The first `token()` lazily initializes the cache. Thereafter an idle background
worker refreshes without needing a caller. `expires_in` must be positive whole
seconds (JSON integer or decimal string); missing, malformed and overflowing
lifetimes fail closed. Expiry is anchored before network I/O. The proactive delay
is `min(3000 seconds, lifetime - lifetime / 6)`, preserving a one-sixth margin for
short lifetimes. Lifetime metadata determines actual expiry; one hour is not a
default. A response already past its proactive deadline is rejected. A held token
also checks its own actual expiry when producing an authorization header.

Concurrent misses/refreshes use one refresh mutex with a second cache check.
The RwLock protects short cache operations only; it is released before HTTP I/O
and before application work. The refresh mutex never covers a financial request.
A failed refresh discards the old cache, returns a redacted error and shares a
five-second retry backoff among callers and the background worker. There is no
fallback to an expired, rejected or possibly invalidated token. Background retries
continue at that bounded rate until recovery or cancellation.

For a provider request, obtain an `AuthorizationAttempt` immediately before I/O.
Its `token()` returns an optional token; `authorization()` produces a sensitive
header. On HTTP 401 call `refresh_after_unauthorized()` **at most once** for that
same attempt. It invalidates only the rejected token generation; concurrent/stale
401s reuse a newer generation. A failed refresh leaves the attempt without a
token. A second call returns `RetryLimit`. A successful refresh only prepares a
header: it does not replay the HTTP request. Later financial transports must
independently decide whether any replay is safe under the dispatch/evidence gates.
Transport errors and timeouts do not themselves authorize financial retries.

`cancel()` cancels that actor and every clone, interrupting pending HTTP I/O,
refresh waiters and background work. Cancellation is terminal. Dropping the last
actor/authorization attempt also cancels background work. Dropping an individual
request future releases its refresh lock; subsequent calls can recover. Previously
issued token snapshots cannot be recalled from external holders: callers must
stop their own work on shutdown and obtain a fresh attempt for each new request.

`crypto::Secret` redacts Debug, has no Display/Serialize implementation and clears
its owned storage on drop. `expose()` is an explicit wire boundary, not a logging
API. The library's config Debug, token Debug, errors and trace output omit secret
values. HTTP authorization headers are marked sensitive. Wire/library buffers and
external callers can still make copies; this is not a claim of process-wide memory
erasure or protection against callers deliberately logging exposed strings.

`stk_password` uses an injected `Clock` and UTC+03:00, returning the matching
14-digit timestamp and Base64(shortcode + passkey + timestamp). `SystemClock`
provides production UTC time; tests exercise fixed timestamps and year rollover.

`CertificateConfig` selects exactly the configured sandbox or production path,
with no fallback. `SecurityCredentialKey` parses a DER or PEM X.509 certificate,
extracts its RSA SubjectPublicKeyInfo and tags the key with the selected environment.
Encryption requires that same environment, uses OS randomness and RSA PKCS#1 v1.5,
then Base64-encodes the ciphertext. The plaintext is the unencoded initiator password
bytes. Randomized ciphertexts must be checked by decryption, not byte equality.
Malformed certificates, non-RSA keys, missing files, environment mismatch and
oversized plaintext fail with redacted errors.

Certificate parsing is not chain, expiry, revocation or provider-authenticity
verification. Operators must authenticate the downloaded certificate and bind the
correct file to each environment. No real Safaricom certificates or credentials
are bundled; all committed fixture keys/certificates are public synthetic test data.
Provider access and live acceptance remain separate disabled capability gates.

Sources checked 2026-10-20:

- Source specification, sections 4, 5.1 and phase 1, checked against the locally maintained original PDF.
- [Safaricom Authorization](https://developer.safaricom.co.ke/apis/Authorization): Basic authentication, grant type, one-hour description and `expires_in` metadata. It also warns a newly generated token invalidates the previous token.
- [Safaricom Getting Started](https://developer.safaricom.co.ke/apis/GettingStarted) and [Transaction Status](https://developer.safaricom.co.ke/apis/TransactionStatus): the explicit algorithm encrypts unencrypted password bytes with RSA PKCS#1 v1.5, then Base64 encodes ciphertext. A prose reference to a base64-encoded input in Getting Started is inconsistent with its numbered algorithm; implementation follows the algorithm and Transaction Status instructions.
- [Safaricom FAQ](https://developer.safaricom.co.ke/faqs): separate sandbox/production certificate downloads.
- Exact installed source: reqwest 0.12.24 `ClientBuilder::{redirect,retry,timeout}`, `retry::never`, `RequestBuilder::basic_auth` marks headers sensitive, `Response::chunk` needs neither json nor stream features; x509-cert 0.2.5 `Certificate::{from_der,from_pem}` / `TbsCertificateInner::subject_public_key_info`; rsa 0.9.10 SPKI decoding and `Pkcs1v15Encrypt` with `rand_core::OsRng`. Consulted versioned docs.rs pages and resolved unavailable exact pages against installed source.
- RustCrypto rsa's documented private-operation timing advisory concerns decryption/signing. Production code here performs public-key encryption only; private-key decryption exists solely in synthetic local tests.
