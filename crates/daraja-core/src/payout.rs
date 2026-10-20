//! Payout wire adapters and untrusted asynchronous observations. No financial retries.
use crate::{
    amount::Amount,
    auth::DisbursementApp,
    collection::{StatusAcknowledgment, StatusCredentials, StatusResult, StatusTimeout},
    crypto::{Clock, Environment, Secret, SecurityCredentialKey},
    identity::{Id, ReceiptId},
    transport::{
        ClientError, ErrorKind, HttpTransport, Outcome, Result, authorized_send, callback_url,
        required_id, validate_base,
    },
};
use reqwest::Url;
use serde_json::{Value, json};
use std::{collections::BTreeMap, future::Future};

#[derive(Clone, Debug)]
pub enum Destination {
    Phone(Secret),
    Till(Id),
    PayBill {
        shortcode: Id,
        account_reference: Secret,
    },
}
impl Destination {
    pub fn parse(input: &str) -> Result<Self> {
        if let Some(till) = input.strip_prefix("TILL:") {
            digits(till, 5, 7)?;
            return Ok(Self::Till(id(till)?));
        }
        if let Some(paybill) = input.strip_prefix("PAYBILL:") {
            let (shortcode, account) = paybill.split_once(':').ok_or_else(input_error)?;
            digits(shortcode, 5, 6)?;
            text(account, 1, 13)?;
            if !account
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(input_error());
            }
            return Ok(Self::PayBill {
                shortcode: id(shortcode)?,
                account_reference: Secret::new(account),
            });
        }
        let phone = if input.len() == 10 && (input.starts_with("07") || input.starts_with("01")) {
            format!("254{}", &input[1..])
        } else {
            input.to_owned()
        };
        digits(&phone, 12, 12)?;
        if !phone.starts_with("2547") && !phone.starts_with("2541") {
            return Err(input_error());
        }
        Ok(Self::Phone(Secret::new(phone)))
    }
}

/// Operator attestations are endpoint/environment specific, never established by mocks.
#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub phone_01: bool,
    pub buy_goods: bool,
    pub pay_bill: bool,
    pub b2b_caller_id: bool,
    pub linked_account_sweep: bool,
    pub transfer_mmf_to_utility: bool,
}
#[derive(Debug)]
pub struct LinkedAccounts {
    pub source: Id,
    pub utility: Id,
}
#[derive(Debug)]
pub struct Remitter {
    initiator: Secret,
    credential: Secret,
    shortcode: Id,
    environment: Environment,
}
impl Remitter {
    pub fn new(
        environment: Environment,
        key: &SecurityCredentialKey,
        initiator: Secret,
        password: &Secret,
        shortcode: Id,
    ) -> Result<Self> {
        digits(shortcode.as_str(), 5, 7)?;
        text(initiator.expose(), 1, 100)?;
        if password.expose().is_empty() {
            return Err(input_error());
        }
        let credential = key
            .encrypt(environment, password)
            .map_err(|_| ClientError::new(ErrorKind::Config))?;
        Ok(Self {
            initiator,
            credential,
            shortcode,
            environment,
        })
    }
    fn body(&self) -> Value {
        json!({"SecurityCredential":self.credential.expose(), "PartyA":self.shortcode.as_str()})
    }
}
#[derive(Debug)]
pub struct PayoutRequest {
    pub caller_originator_conversation_id: Id,
    pub destination: Destination,
    pub amount: Amount,
    pub result_url: Secret,
    pub timeout_url: Secret,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Command {
    BusinessPayment,
    BusinessBuyGoods,
    BusinessPayBill,
    BusinessTransferFromMMFToUtility,
}
impl Command {
    fn wire(self) -> &'static str {
        match self {
            Self::BusinessPayment => "BusinessPayment",
            Self::BusinessBuyGoods => "BusinessBuyGoods",
            Self::BusinessPayBill => "BusinessPayBill",
            Self::BusinessTransferFromMMFToUtility => "BusinessTransferFromMMFToUtility",
        }
    }
    fn path(self) -> &'static str {
        if self == Self::BusinessPayment {
            "/mpesa/b2c/v3/paymentrequest"
        } else {
            "/mpesa/b2b/v1/paymentrequest"
        }
    }
}
#[derive(Debug)]
pub struct PreparedPayout {
    pub caller_originator_conversation_id: Id,
    pub command: Command,
    pub wire_body: Secret,
}
#[derive(Debug)]
pub struct PayoutAcknowledgment {
    pub caller_originator_conversation_id: Id,
    pub provider: StatusAcknowledgment,
}
/// Commit immutable identity/body before I/O and atomically bind distinct provider IDs.
/// Failure/cancellation leaves the prepared operation unresolved; never infer non-dispatch.
pub trait PayoutPersistence: Send + Sync {
    fn prepare(&self, payout: &PreparedPayout) -> impl Future<Output = Result<()>> + Send;
    fn acknowledge(&self, ack: &PayoutAcknowledgment) -> impl Future<Output = Result<()>> + Send;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    NotDispatched,
    Duplicate,
    ProvenRejection,
    Uncertain,
}
#[derive(Debug)]
pub struct PayoutError {
    pub caller_originator_conversation_id: Id,
    pub disposition: Disposition,
    pub source: ClientError,
}
impl std::fmt::Display for PayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "payout transport: {:?}", self.disposition)
    }
}
impl std::error::Error for PayoutError {}

pub struct PayoutClient<T> {
    transport: T,
    base: Url,
    app: DisbursementApp,
    remitter: Remitter,
    capabilities: Capabilities,
    linked_accounts: Option<LinkedAccounts>,
    max_amount_kes: u64,
    proven_rejection_codes: BTreeMap<Command, Vec<Id>>,
}
impl<T: HttpTransport> PayoutClient<T> {
    /// max_amount_kes is an operator ceiling bounded by the approved product/account limit.
    pub fn new(
        transport: T,
        base: Url,
        app: DisbursementApp,
        remitter: Remitter,
        max_amount_kes: u64,
    ) -> Result<Self> {
        environment_base(&base, remitter.environment)?;
        digits(remitter.shortcode.as_str(), 5, 6)?;
        if max_amount_kes == 0 || !app.matches_origin(&base) {
            return Err(ClientError::new(ErrorKind::Config));
        }
        Ok(Self {
            transport,
            base,
            app,
            remitter,
            max_amount_kes,
            capabilities: Capabilities::default(),
            linked_accounts: None,
            proven_rejection_codes: BTreeMap::new(),
        })
    }
    pub fn configure_capabilities(
        &mut self,
        capabilities: Capabilities,
        linked_accounts: Option<LinkedAccounts>,
    ) -> Result<()> {
        if let Some(linked) = &linked_accounts {
            digits(linked.source.as_str(), 5, 6)?;
            digits(linked.utility.as_str(), 5, 6)?;
            if linked.source != self.remitter.shortcode || linked.source == linked.utility {
                return Err(ClientError::new(ErrorKind::Config));
            }
        }
        self.capabilities = capabilities;
        self.linked_accounts = linked_accounts;
        Ok(())
    }
    /// Only codes independently verified to prove non-execution for this endpoint/environment.
    /// HTTP 5xx, duplicate, throttling and missing/malformed responses remain unresolved.
    pub fn configure_proven_rejections(&mut self, command: Command, codes: Vec<Id>) -> Result<()> {
        if codes
            .iter()
            .any(|c| matches!(c.as_str(), "0" | "500.002.1001" | "500.001.1001"))
        {
            return Err(ClientError::new(ErrorKind::Config));
        }
        self.proven_rejection_codes.insert(command, codes);
        Ok(())
    }
    fn build(&self, request: &PayoutRequest, sweep: bool) -> Result<(Command, Value)> {
        callback_url(&request.result_url)?;
        callback_url(&request.timeout_url)?;
        text(request.caller_originator_conversation_id.as_str(), 1, 64)?;
        let amount = request.amount.whole_kes().map_err(|_| input_error())?;
        if amount == 0 || amount > self.max_amount_kes {
            return Err(input_error());
        }
        let destination = match &request.destination {
            Destination::Phone(phone) => Destination::parse(phone.expose())?,
            Destination::Till(till) => Destination::parse(&format!("TILL:{}", till.as_str()))?,
            Destination::PayBill {
                shortcode,
                account_reference,
            } => Destination::parse(&format!(
                "PAYBILL:{}:{}",
                shortcode.as_str(),
                account_reference.expose()
            ))?,
        };
        let mut body = self.remitter.body();
        let command = match &destination {
            Destination::Phone(phone) if !sweep => {
                if phone.expose().starts_with("2541") && !self.capabilities.phone_01 {
                    return Err(disabled());
                }
                body["PartyB"] = json!(phone.expose());
                body["InitiatorName"] = json!(self.remitter.initiator.expose());
                body["Occassion"] = json!("Payout");
                Command::BusinessPayment
            }
            Destination::Till(till) if !sweep => {
                if !self.capabilities.buy_goods || !self.capabilities.b2b_caller_id {
                    return Err(disabled());
                }
                body["PartyB"] = json!(till.as_str());
                body["RecieverIdentifierType"] = json!(2);
                body["AccountReference"] = json!("Payout");
                Command::BusinessBuyGoods
            }
            Destination::PayBill {
                shortcode,
                account_reference,
            } => {
                let command = if sweep {
                    let linked = self.linked_accounts.as_ref().ok_or_else(disabled)?;
                    if !self.capabilities.linked_account_sweep
                        || !self.capabilities.transfer_mmf_to_utility
                        || linked.utility != *shortcode
                    {
                        return Err(disabled());
                    }
                    Command::BusinessTransferFromMMFToUtility
                } else {
                    if !self.capabilities.pay_bill {
                        return Err(disabled());
                    }
                    Command::BusinessPayBill
                };
                if !self.capabilities.b2b_caller_id {
                    return Err(disabled());
                }
                body["PartyB"] = json!(shortcode.as_str());
                body["RecieverIdentifierType"] = json!(4);
                body["AccountReference"] = json!(account_reference.expose());
                command
            }
            _ => return Err(input_error()),
        };
        if command != Command::BusinessPayment {
            body["Initiator"] = json!(self.remitter.initiator.expose());
            body["SenderIdentifierType"] = json!(4);
        }
        body["OriginatorConversationID"] =
            json!(request.caller_originator_conversation_id.as_str());
        body["CommandID"] = json!(command.wire());
        body["Amount"] = json!(amount);
        body["Remarks"] = json!("Corporate payout");
        body["ResultURL"] = json!(request.result_url.expose());
        body["QueueTimeOutURL"] = json!(request.timeout_url.expose());
        Ok((command, body))
    }
    pub async fn submit(
        &self,
        request: &PayoutRequest,
        persistence: &impl PayoutPersistence,
    ) -> std::result::Result<PayoutAcknowledgment, PayoutError> {
        self.dispatch(request, persistence, false).await
    }
    pub async fn sweep(
        &self,
        request: &PayoutRequest,
        persistence: &impl PayoutPersistence,
    ) -> std::result::Result<PayoutAcknowledgment, PayoutError> {
        self.dispatch(request, persistence, true).await
    }
    async fn dispatch(
        &self,
        request: &PayoutRequest,
        persistence: &impl PayoutPersistence,
        sweep: bool,
    ) -> std::result::Result<PayoutAcknowledgment, PayoutError> {
        let failure = |source: ClientError, disposition| PayoutError {
            caller_originator_conversation_id: request.caller_originator_conversation_id.clone(),
            disposition,
            source,
        };
        let (command, body) = self
            .build(request, sweep)
            .map_err(|e| failure(e, Disposition::NotDispatched))?;
        let prepared = PreparedPayout {
            caller_originator_conversation_id: request.caller_originator_conversation_id.clone(),
            command,
            wire_body: Secret::new(body.to_string()),
        };
        persistence.prepare(&prepared).await.map_err(|_| {
            failure(
                ClientError::new(ErrorKind::Persistence),
                Disposition::NotDispatched,
            )
        })?;
        let (value, outcome) =
            authorized_send(&self.app, &self.transport, &self.base, command.path(), body)
                .await
                .map_err(|e| {
                    let code = e.outcome.provider_code.as_ref();
                    let disposition = if code.is_some_and(|c| c.as_str() == "500.002.1001") {
                        Disposition::Duplicate
                    } else if e
                        .outcome
                        .http_status
                        .is_some_and(|s| (200..500).contains(&s))
                        && code.is_some_and(|c| {
                            self.proven_rejection_codes
                                .get(&command)
                                .is_some_and(|codes| codes.contains(c))
                        })
                    {
                        Disposition::ProvenRejection
                    } else {
                        Disposition::Uncertain
                    };
                    failure(e, disposition)
                })?;
        let provider =
            acknowledgment(value, outcome).map_err(|e| failure(e, Disposition::Uncertain))?;
        let ack = PayoutAcknowledgment {
            caller_originator_conversation_id: prepared.caller_originator_conversation_id,
            provider,
        };
        persistence.acknowledge(&ack).await.map_err(|_| {
            failure(
                ClientError {
                    kind: ErrorKind::Persistence,
                    outcome: Box::new(ack.provider.outcome.clone()),
                },
                Disposition::Uncertain,
            )
        })?;
        Ok(ack)
    }
}

#[derive(Clone, Debug)]
pub enum BalanceAccount {
    Till(Id),
    Shortcode(Id),
}
impl BalanceAccount {
    fn parts(&self) -> Result<(&Id, u8)> {
        let (account, kind, max) = match self {
            Self::Till(id) => (id, 2, 7),
            Self::Shortcode(id) => (id, 4, 6),
        };
        digits(account.as_str(), 5, max)?;
        Ok((account, kind))
    }
}
#[derive(Debug)]
pub struct BalanceRequest {
    pub local_query_id: Id,
    pub account: BalanceAccount,
    pub result_url: Secret,
    pub timeout_url: Secret,
}
#[derive(Debug)]
pub struct BalancePrepared<'a> {
    pub request: &'a BalanceRequest,
    pub wire_body: Secret,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
/// Persist the query binding and scheduled unresolved work before I/O; bind provider IDs
/// without overwriting early callback observations. No no-op production implementation.
pub trait BalancePersistence: Send + Sync {
    fn prepare(&self, record: BalancePrepared<'_>) -> impl Future<Output = Result<()>> + Send;
    fn acknowledge(
        &self,
        local_query_id: &Id,
        ack: &StatusAcknowledgment,
    ) -> impl Future<Output = Result<()>> + Send;
}
pub struct BalanceClient<T, C> {
    transport: T,
    base: Url,
    credentials: StatusCredentials,
    remitter: Remitter,
    clock: C,
    till_enabled: bool,
}
impl<T: HttpTransport, C: Clock> BalanceClient<T, C> {
    pub fn new(
        transport: T,
        base: Url,
        credentials: StatusCredentials,
        remitter: Remitter,
        clock: C,
    ) -> Result<Self> {
        environment_base(&base, remitter.environment)?;
        let matched = match &credentials {
            StatusCredentials::Collection(app) => app.matches_origin(&base),
            StatusCredentials::Disbursement(app) => app.matches_origin(&base),
        };
        if !matched {
            return Err(ClientError::new(ErrorKind::Config));
        }
        Ok(Self {
            transport,
            base,
            credentials,
            remitter,
            clock,
            till_enabled: false,
        })
    }
    /// Requires verified IdentifierType=2 support for the configured account/environment.
    pub fn enable_till_balance(&mut self) {
        self.till_enabled = true;
    }
    pub async fn request(
        &self,
        request: &BalanceRequest,
        persistence: &impl BalancePersistence,
    ) -> Result<StatusAcknowledgment> {
        let (account, kind) = request.account.parts()?;
        if kind == 2 && !self.till_enabled {
            return Err(disabled());
        }
        if account != &self.remitter.shortcode {
            return Err(input_error());
        }
        callback_url(&request.result_url)?;
        callback_url(&request.timeout_url)?;
        let mut body = self.remitter.body();
        body["Initiator"] = json!(self.remitter.initiator.expose());
        body["CommandID"] = json!("AccountBalance");
        body["IdentifierType"] = json!(kind);
        body["Remarks"] = json!("Balance query");
        body["ResultURL"] = json!(request.result_url.expose());
        body["QueueTimeOutURL"] = json!(request.timeout_url.expose());
        persistence
            .prepare(BalancePrepared {
                request,
                wire_body: Secret::new(body.to_string()),
                created_at: self.clock.now(),
            })
            .await
            .map_err(|_| ClientError::new(ErrorKind::Persistence))?;
        let path = "/mpesa/accountbalance/v1/query";
        let (value, outcome) = match &self.credentials {
            StatusCredentials::Collection(app) => {
                authorized_send(app, &self.transport, &self.base, path, body).await
            }
            StatusCredentials::Disbursement(app) => {
                authorized_send(app, &self.transport, &self.base, path, body).await
            }
        }?;
        let ack = acknowledgment(value, outcome)?;
        persistence
            .acknowledge(&request.local_query_id, &ack)
            .await
            .map_err(|_| ClientError {
                kind: ErrorKind::Persistence,
                outcome: Box::new(ack.outcome.clone()),
            })?;
        Ok(ack)
    }
}

/// Includes redacted AccountBalance parameters; interpretation/admission belongs to liquidity.
pub type BalanceResult = StatusResult;
pub type BalanceTimeout = StatusTimeout;
pub type PayoutTimeout = StatusTimeout;
#[derive(Debug)]
pub struct PayoutResult {
    pub observation: StatusResult,
    pub receipt: Option<ReceiptId>,
    pub amount: Option<Amount>,
}
impl PayoutResult {
    pub fn parse(body: &str) -> Result<Self> {
        let observation = StatusResult::parse(body)?;
        let invalid = || ClientError::new(ErrorKind::InvalidResponse);
        let mut receipt = observation.receipt.clone();
        if let Some(transaction) = &observation.transaction_id {
            receipt = Some(ReceiptId::new(transaction.as_str()).map_err(|_| invalid())?);
        }
        if let Some(value) = observation.parameters.get("TransactionReceipt") {
            let reported = ReceiptId::new(value.expose()).map_err(|_| invalid())?;
            if receipt.as_ref().is_some_and(|r| r != &reported) {
                return Err(invalid());
            }
            receipt = Some(reported);
        }
        let mut amount = observation.observed_amount;
        if let Some(value) = observation.parameters.get("TransactionAmount") {
            let reported =
                crate::collection::parse_kes(&json!(value.expose())).map_err(|_| invalid())?;
            if amount.is_some_and(|a| a != reported) {
                return Err(invalid());
            }
            amount = Some(reported);
        }
        Ok(Self {
            observation,
            receipt,
            amount,
        })
    }
}
fn acknowledgment(value: Value, outcome: Outcome) -> Result<StatusAcknowledgment> {
    Ok(StatusAcknowledgment {
        originator_conversation_id: required_id(&value, "OriginatorConversationID", &outcome)?,
        conversation_id: required_id(&value, "ConversationID", &outcome)?,
        outcome,
    })
}
fn input_error() -> ClientError {
    ClientError::new(ErrorKind::Input)
}
fn disabled() -> ClientError {
    ClientError::new(ErrorKind::CapabilityDisabled)
}
fn id(s: &str) -> Result<Id> {
    Id::new(s).map_err(|_| input_error())
}
fn digits(s: &str, min: usize, max: usize) -> Result<()> {
    if !(min..=max).contains(&s.len())
        || s.starts_with('0')
        || !s.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(input_error());
    }
    Ok(())
}
fn text(s: &str, min: usize, max: usize) -> Result<()> {
    if !(min..=max).contains(&s.chars().count()) || s.trim() != s || s.chars().any(char::is_control)
    {
        return Err(input_error());
    }
    Ok(())
}
fn environment_base(base: &Url, environment: Environment) -> Result<()> {
    validate_base(base)?;
    let expected = match environment {
        Environment::Sandbox => "sandbox.safaricom.co.ke",
        Environment::Production => "api.safaricom.co.ke",
    };
    let local = base.host_str().is_some_and(|h| {
        h.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    });
    if base.host_str() != Some(expected) && !local {
        return Err(ClientError::new(ErrorKind::Config));
    }
    Ok(())
}
