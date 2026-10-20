use axum::{
    Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use daraja_core::{
    auth::{AuthError, CollectionApp, DisbursementApp, OAuthConfig},
    crypto::Secret,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, RwLock};

#[derive(Clone)]
enum Reply {
    Token(Value),
    Reject,
    Hang,
    DelayedToken(Value),
    Redirect,
}
struct Provider {
    counts: [AtomicUsize; 2],
    replies: RwLock<[Reply; 2]>,
    headers: Mutex<Vec<(usize, String)>>,
    release: Notify,
}
struct Server {
    state: Arc<Provider>,
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(
    State(state): State<Arc<Provider>>,
    Path(app): Path<usize>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    assert_eq!(
        query.get("grant_type").map(String::as_str),
        Some("client_credentials")
    );
    state.counts[app].fetch_add(1, Ordering::SeqCst);
    state
        .headers
        .lock()
        .unwrap()
        .push((app, headers["authorization"].to_str().unwrap().to_string()));
    let reply = state.replies.read().await[app].clone();
    match reply {
        Reply::Token(value) => axum::Json(value).into_response(),
        Reply::Reject => {
            (StatusCode::UNAUTHORIZED, "synthetic-secret-in-error-body").into_response()
        }
        Reply::DelayedToken(value) => {
            state.release.notified().await;
            axum::Json(value).into_response()
        }
        Reply::Hang => {
            state.release.notified().await;
            axum::Json(json!({"access_token":"released", "expires_in":3600})).into_response()
        }
        Reply::Redirect => Response::builder()
            .status(302)
            .header("location", "/1")
            .body(Body::empty())
            .unwrap(),
    }
}
impl Server {
    async fn new() -> Self {
        let state = Arc::new(Provider {
            counts: [AtomicUsize::new(0), AtomicUsize::new(0)],
            replies: RwLock::new([
                token("collection-token", json!(3600)),
                token("disbursement-token", json!("3600")),
            ]),
            headers: Mutex::new(Vec::new()),
            release: Notify::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/{app}", get(serve))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { state, url, task }
    }
    fn config(&self, app: usize) -> OAuthConfig {
        OAuthConfig {
            endpoint: format!("{}/{app}", self.url).parse().unwrap(),
            consumer_key: Secret::new(format!("key-{app}")),
            consumer_secret: Secret::new(format!("secret-{app}")),
        }
    }
    fn count(&self, app: usize) -> usize {
        self.state.counts[app].load(Ordering::SeqCst)
    }
    async fn reply(&self, app: usize, reply: Reply) {
        self.state.replies.write().await[app] = reply;
    }
    async fn wait_count(&self, app: usize, count: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.count(app) < count {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    }
}
fn token(value: &str, expiry: Value) -> Reply {
    Reply::Token(json!({"access_token":value, "expires_in":expiry}))
}
async fn advance(seconds: u64) {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(seconds)).await;
    tokio::time::resume();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn isolated_apps_and_concurrent_single_flight() {
    let server = Server::new().await;
    let collection = CollectionApp::new(server.config(0)).unwrap();
    let disbursement = DisbursementApp::new(server.config(1)).unwrap();
    let mut tasks = Vec::new();
    for _ in 0..40 {
        let actor = collection.clone();
        tasks.push(tokio::spawn(async move {
            actor.token().await.unwrap().authorization().unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), "Bearer collection-token");
    }
    assert_eq!(
        disbursement.token().await.unwrap().authorization().unwrap(),
        "Bearer disbursement-token"
    );
    assert_eq!((server.count(0), server.count(1)), (1, 1));
    let mut attempt = collection.authorize().await.unwrap();
    attempt.refresh_after_unauthorized().await.unwrap();
    assert_eq!((server.count(0), server.count(1)), (2, 1));
    assert_eq!(
        attempt.refresh_after_unauthorized().await,
        Err(AuthError::RetryLimit)
    );
    for (app, header) in server.state.headers.lock().unwrap().iter() {
        assert_eq!(
            header,
            &format!(
                "Basic {}",
                STANDARD.encode(format!("key-{app}:secret-{app}"))
            )
        );
    }
}

#[tokio::test]
async fn proactive_background_refresh_uses_metadata_and_fifty_minute_target() {
    let server = Server::new().await;
    let actor = CollectionApp::new(server.config(0)).unwrap();
    actor.token().await.unwrap();
    advance(2999).await;
    assert_eq!(
        actor.token().await.unwrap().authorization().unwrap(),
        "Bearer collection-token"
    );
    assert_eq!(server.count(0), 1);
    server.reply(0, token("refreshed", json!(120))).await;
    advance(1).await;
    server.wait_count(0, 2).await;
    assert_eq!(
        actor.token().await.unwrap().authorization().unwrap(),
        "Bearer refreshed"
    );
    server.reply(0, token("short-refreshed", json!(120))).await;
    advance(100).await;
    server.wait_count(0, 3).await;
    assert_eq!(
        actor.token().await.unwrap().authorization().unwrap(),
        "Bearer short-refreshed"
    );
}

#[tokio::test]
async fn failure_is_shared_expired_tokens_are_never_returned_and_recovery_works() {
    let server = Server::new().await;
    server.reply(0, token("short-token", json!(6))).await;
    let actor = CollectionApp::new(server.config(0)).unwrap();
    actor.token().await.unwrap();
    server.reply(0, Reply::Reject).await;
    advance(6).await;
    let mut tasks = Vec::new();
    for _ in 0..30 {
        let actor = actor.clone();
        tasks.push(tokio::spawn(
            async move { actor.token().await.unwrap_err() },
        ));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), AuthError::Rejected);
    }
    assert_eq!(server.count(0), 2);
    server.reply(0, token("recovered", json!(3600))).await;
    advance(5).await;
    assert_eq!(
        actor.token().await.unwrap().authorization().unwrap(),
        "Bearer recovered"
    );
    assert_eq!(server.count(0), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_401s_refresh_once_and_stale_401_does_not_evict_new_token() {
    let server = Server::new().await;
    let actor = CollectionApp::new(server.config(0)).unwrap();
    let mut attempts = Vec::new();
    for _ in 0..30 {
        attempts.push(actor.authorize().await.unwrap());
    }
    let mut stale = actor.authorize().await.unwrap();
    server.reply(0, token("new-generation", json!(3600))).await;
    let tasks: Vec<_> = attempts
        .into_iter()
        .map(|mut attempt| {
            tokio::spawn(async move {
                attempt.refresh_after_unauthorized().await.unwrap();
                attempt.token().unwrap().authorization().unwrap()
            })
        })
        .collect();
    for task in tasks {
        assert_eq!(task.await.unwrap(), "Bearer new-generation");
    }
    stale.refresh_after_unauthorized().await.unwrap();
    assert_eq!(server.count(0), 2);
}

#[tokio::test]
async fn failed_401_refresh_does_not_resurrect_rejected_cache() {
    let server = Server::new().await;
    let actor = CollectionApp::new(server.config(0)).unwrap();
    let mut attempt = actor.authorize().await.unwrap();
    server.reply(0, Reply::Reject).await;
    assert_eq!(
        attempt.refresh_after_unauthorized().await,
        Err(AuthError::Rejected)
    );
    assert_eq!(
        attempt.refresh_after_unauthorized().await,
        Err(AuthError::RetryLimit)
    );
    assert_eq!(actor.token().await.unwrap_err(), AuthError::Rejected);
    assert!(attempt.token().is_none());
    advance(5).await;
    server.reply(0, token("recovered", json!(3600))).await;
    assert_eq!(
        actor.token().await.unwrap().authorization().unwrap(),
        "Bearer recovered"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_interrupts_refresh_and_waiters_without_affecting_other_app() {
    let server = Server::new().await;
    server.reply(0, Reply::Hang).await;
    let collection = CollectionApp::new(server.config(0)).unwrap();
    let disbursement = DisbursementApp::new(server.config(1)).unwrap();
    let mut tasks = Vec::new();
    for _ in 0..20 {
        let actor = collection.clone();
        tasks.push(tokio::spawn(
            async move { actor.token().await.unwrap_err() },
        ));
    }
    server.wait_count(0, 1).await;
    collection.cancel();
    for task in tasks {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap(),
            AuthError::Cancelled
        );
    }
    assert_eq!(collection.token().await.unwrap_err(), AuthError::Cancelled);
    assert_eq!(
        disbursement.token().await.unwrap().authorization().unwrap(),
        "Bearer disbursement-token"
    );
    server.state.release.notify_waiters();
    assert_eq!(server.count(0), 1);
}

#[tokio::test]
async fn dropping_request_during_refresh_allows_later_recovery() {
    let server = Server::new().await;
    let actor = CollectionApp::new(server.config(0)).unwrap();
    actor.token().await.unwrap();
    server.reply(0, Reply::Hang).await;
    let mut attempt = actor.authorize().await.unwrap();
    let task = tokio::spawn(async move { attempt.refresh_after_unauthorized().await });
    server.wait_count(0, 2).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    server.reply(0, token("after-abort", json!(3600))).await;
    assert_eq!(
        actor.token().await.unwrap().authorization().unwrap(),
        "Bearer after-abort"
    );
    server.state.release.notify_waiters();
}

#[tokio::test]
async fn dropping_last_actor_cancels_background_refresh() {
    let server = Server::new().await;
    let actor = CollectionApp::new(server.config(0)).unwrap();
    actor.token().await.unwrap();
    let copy = actor.clone();
    drop(actor);
    assert!(copy.token().await.is_ok());
    drop(copy);
    advance(3600).await;
    tokio::task::yield_now().await;
    assert_eq!(server.count(0), 1);
}

#[tokio::test]
async fn malformed_expiry_token_and_oversized_responses_fail_closed() {
    let server = Server::new().await;
    for response in [
        json!({"access_token":"x"}),
        json!({"access_token":"x", "expires_in":0}),
        json!({"access_token":"x", "expires_in":-1}),
        json!({"access_token":"x", "expires_in":"nonsense"}),
        json!({"access_token":"x", "expires_in":1.5}),
        json!({"access_token":"", "expires_in":3600}),
        json!({"access_token":"bad\r\nheader", "expires_in":3600}),
        json!({"access_token":"x", "expires_in":u64::MAX}),
        json!({"access_token":"x".repeat(17000), "expires_in":3600}),
    ] {
        server.reply(0, Reply::Token(response)).await;
        let actor = CollectionApp::new(server.config(0)).unwrap();
        assert_eq!(actor.token().await.unwrap_err(), AuthError::InvalidResponse);
    }
}

#[tokio::test]
async fn no_redirects_transport_timeout_and_recovery() {
    let server = Server::new().await;
    server.reply(0, Reply::Redirect).await;
    let actor = CollectionApp::new(server.config(0)).unwrap();
    assert_eq!(actor.token().await.unwrap_err(), AuthError::Rejected);
    assert_eq!(server.count(1), 0);
    server.reply(0, Reply::Hang).await;
    advance(5).await;
    let copy = actor.clone();
    let task = tokio::spawn(async move { copy.token().await });
    server.wait_count(0, 2).await;
    advance(16).await;
    assert_eq!(task.await.unwrap().unwrap_err(), AuthError::Transport);
    server.reply(0, token("after-timeout", json!(3600))).await;
    advance(5).await;
    assert_eq!(
        actor.token().await.unwrap().authorization().unwrap(),
        "Bearer after-timeout"
    );
    server.state.release.notify_waiters();
}

#[tokio::test]
async fn unsafe_endpoints_are_rejected_and_config_debug_is_redacted() {
    let server = Server::new().await;
    for endpoint in [
        "http://example.com/oauth",
        "https://url-user:url-secret@example.com/oauth",
        "https://example.com/oauth?secret=passkey",
        "https://example.com/oauth#secret",
    ] {
        let mut config = server.config(0);
        config.endpoint = endpoint.parse().unwrap();
        let debug = format!("{config:?}");
        for secret in ["key-0", "secret-0", "url-user", "url-secret", "passkey"] {
            assert!(!debug.contains(secret));
        }
        assert_eq!(
            CollectionApp::new(config).unwrap_err(),
            AuthError::InvalidConfig
        );
    }
}

#[tokio::test]
async fn held_token_expires_and_delayed_response_does_not_extend_lifetime() {
    let server = Server::new().await;
    server.reply(0, token("held-token", json!(6))).await;
    let actor = CollectionApp::new(server.config(0)).unwrap();
    let held = actor.token().await.unwrap();
    server.reply(0, Reply::Reject).await;
    advance(6).await;
    assert_eq!(held.authorization().unwrap_err(), AuthError::ExpiredToken);
    actor.cancel();
    assert_eq!(actor.token().await.unwrap_err(), AuthError::Cancelled);

    server
        .reply(
            1,
            Reply::DelayedToken(json!({"access_token":"delayed", "expires_in":1})),
        )
        .await;
    let disbursement = DisbursementApp::new(server.config(1)).unwrap();
    let copy = disbursement.clone();
    let task = tokio::spawn(async move { copy.token().await });
    server.wait_count(1, 1).await;
    advance(2).await;
    server.state.release.notify_one();
    assert_eq!(task.await.unwrap().unwrap_err(), AuthError::InvalidResponse);
}
