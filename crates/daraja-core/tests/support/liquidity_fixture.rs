use daraja_core::{
    Timestamp,
    amount::{Amount, Unit},
    collection::StatusAcknowledgment,
    crypto::Secret,
    identity::Id,
    liquidity::{AccountRole, BalanceEvidence, LiquidityPolicy},
    payout::{BalanceAccount, BalancePrepared, BalanceRequest},
    repository::Repository,
    transport::Outcome,
};
fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}

pub async fn fund(repository: &Repository) {
    if repository.liquidity_policy().await.is_ok() {
        return;
    }
    repository
        .configure_liquidity(LiquidityPolicy {
            collection: id("600001"),
            disbursement: id("600000"),
            minimum: Amount::new(100, Unit::KesMinor),
            max_age_seconds: u32::MAX,
            poll_interval_seconds: 1,
        })
        .await
        .unwrap();
    for (role, account) in [
        (AccountRole::Collection, "600001"),
        (AccountRole::Disbursement, "600000"),
    ] {
        observe(repository, role, account, "initial", 0, 1_000_000_000_000).await;
    }
}

pub async fn prepare(
    repository: &Repository,
    role: AccountRole,
    account: &str,
    name: &str,
    at: u64,
) -> Id {
    let query = id(&format!("{account}-{name}"));
    let request = BalanceRequest {
        local_query_id: query.clone(),
        account: BalanceAccount::Shortcode(id(account)),
        result_url: Secret::new("https://example.invalid/result/fixture"),
        timeout_url: Secret::new("https://example.invalid/timeout/fixture"),
    };
    repository.prepare_balance_query(role, BalancePrepared { request: &request,
        wire_body: Secret::new(serde_json::json!({"PartyA":account,"CommandID":"AccountBalance","IdentifierType":4,
            "ResultURL":request.result_url.expose(),"QueueTimeOutURL":request.timeout_url.expose()}).to_string()),
        created_at: chrono::DateTime::from_timestamp(at.try_into().unwrap(),0).unwrap(),
    }).await.unwrap();
    repository
        .acknowledge_balance(
            &query,
            &StatusAcknowledgment {
                outcome: Outcome::default(),
                originator_conversation_id: id(&format!("origin-{account}-{name}")),
                conversation_id: id(&format!("conv-{account}-{name}")),
            },
        )
        .await
        .unwrap();
    query
}
pub async fn observe(
    repository: &Repository,
    role: AccountRole,
    account: &str,
    name: &str,
    at: u64,
    amount: u64,
) {
    let query = prepare(repository, role, account, name, at).await;
    verify(repository, role, account, query, at, amount).await;
}
pub async fn verify(
    repository: &Repository,
    role: AccountRole,
    account: &str,
    query: Id,
    at: u64,
    amount: u64,
) {
    repository
        .record_verified_balance(
            &query,
            BalanceEvidence {
                query_id: query.clone(),
                account: id(account),
                role,
                available: Amount::new(amount, Unit::KesMinor),
                evidence_record: id("synthetic-reviewed-balance"),
            },
            Timestamp(at),
        )
        .await
        .unwrap();
}
