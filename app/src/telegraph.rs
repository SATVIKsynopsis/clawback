//! Telegraph node client: x402 escrow flow (port of scripts/pay.sh).
//!
//! 1. POST the request with no payment -> 402 with `accepts[]`; take `scheme == "escrow"`.
//! 2. Refuse unless payTo == configured Diamond and network matches; read `Amount: N uUSDC`.
//! 3. Price cap + budget check BEFORE signing.
//! 4. Fill the template ({from} lowercased, fresh {nonce}, validAfter=now-5, validBefore=now+100),
//!    EIP-191 personal_sign it verbatim.
//! 5. Resend with `X-PAYMENT: base64(json)`; receipt comes back in `X-PAYMENT-RESPONSE`.
//!
//! Failed requests are not billed by the node.

use alloy_primitives::hex;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum AskError {
    #[error("no wallet configured (set PRIVATE_KEY); refusing to pay")]
    NoWallet,
    #[error("Telegraph node unreachable: {0}")]
    Unreachable(String),
    #[error("request rejected before payment (HTTP 400): {0}")]
    BadRequest(String),
    #[error("no compatible miner (HTTP 503): {0}")]
    NoCompatibleMiner(String),
    #[error("unexpected payment challenge: {0}")]
    UnexpectedChallenge(String),
    #[error("price {price} uUSDC exceeds max_price_uusdc {max}")]
    PriceTooHigh { price: u64, max: u64 },
    #[error("budget refused {price} uUSDC: {reason}")]
    BudgetDenied { price: u64, reason: String },
    #[error("paid request refused (HTTP {status}, not billed): {reason}")]
    Refused { status: u16, reason: String },
}

impl AskError {
    /// Worth retrying elsewhere (direct ask)? Budget/wallet/network problems are not.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::NoCompatibleMiner(_) | Self::Refused { .. } | Self::BadRequest(_))
    }
}

#[derive(Debug, Clone)]
pub struct Paid {
    pub body: Value,
    pub receipt: Value,
    pub amount_uusdc: u64,
}

impl Paid {
    pub fn receipt_hash(&self) -> String {
        self.receipt.get("receipt_hash").and_then(Value::as_str).unwrap_or_default().to_string()
    }
    pub fn signal_hash(&self) -> String {
        self.body.get("signal_hash").and_then(Value::as_str).unwrap_or_default().to_string()
    }
    pub fn miner_id(&self) -> String {
        match self.body.get("miner_id") {
            Some(Value::String(s)) => s.clone(),
            Some(v) if !v.is_null() => v.to_string(),
            _ => String::new(),
        }
    }
    pub fn miner_name(&self) -> String {
        self.body.get("miner_name").and_then(Value::as_str).unwrap_or_default().to_string()
    }
}

pub struct Client {
    http: reqwest::Client,
    node: String,
    signer: Option<PrivateKeySigner>,
    diamond: String,
    network: String,
    max_price: u64,
}

pub struct Challenge {
    pub template: String,
    pub pay_to: String,
    pub amount: u64,
}

pub fn parse_challenge(body: &Value, diamond: &str, network: &str) -> Result<Challenge, AskError> {
    let bad = |m: &str| AskError::UnexpectedChallenge(m.to_string());
    let escrow = body
        .get("accepts")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|x| x.get("scheme").and_then(Value::as_str) == Some("escrow")))
        .ok_or_else(|| bad("no escrow entry in accepts[]"))?;
    let pay_to = escrow.get("payTo").and_then(Value::as_str).unwrap_or_default().to_lowercase();
    if pay_to != diamond.to_lowercase() {
        return Err(bad(&format!("payTo {pay_to} is not the configured Diamond {diamond}")));
    }
    let net = escrow.get("network").and_then(Value::as_str).unwrap_or_default();
    if net != network {
        return Err(bad(&format!("network {net} != {network}")));
    }
    let template = escrow
        .pointer("/extra/message")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("missing extra.message"))?
        .to_string();
    let amount = template
        .lines()
        .find_map(|l| l.trim().strip_prefix("Amount:")?.trim().strip_suffix("uUSDC")?.trim().parse::<u64>().ok())
        .ok_or_else(|| bad("no 'Amount: N uUSDC' line in template"))?;
    for ph in ["{from}", "{nonce}", "{validAfter}", "{validBefore}"] {
        if !template.contains(ph) {
            return Err(bad(&format!("template lacks {ph}")));
        }
    }
    Ok(Challenge { template, pay_to, amount })
}

fn reason_of(status: u16, text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| {
            ["reason", "error", "message"].iter().find_map(|k| v.get(*k).map(|x| x.as_str().map(str::to_string).unwrap_or(x.to_string())))
        })
        .unwrap_or_else(|| format!("HTTP {status}: {}", text.chars().take(300).collect::<String>()))
}

impl Client {
    pub fn new(node: &str, diamond: &str, network: &str, private_key: Option<&str>, timeout: Duration, max_price: u64) -> Result<Self, String> {
        let signer = match private_key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => Some(k.parse::<PrivateKeySigner>().map_err(|_| "PRIVATE_KEY is not a valid secp256k1 key".to_string())?),
            None => None,
        };
        let http = reqwest::Client::builder()
            .user_agent(concat!("clawback/", env!("CARGO_PKG_VERSION")))
            .timeout(timeout)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            node: node.trim_end_matches('/').to_string(),
            signer,
            diamond: diamond.to_lowercase(),
            network: network.to_string(),
            max_price,
        })
    }

    pub fn address(&self) -> Option<String> {
        self.signer.as_ref().map(|s| format!("{:#x}", s.address()))
    }

    /// Routed ask: the node picks the miner.
    pub async fn ask(&self, intent: &str, params: Value, budget: impl FnOnce(u64) -> Result<(), String>) -> Result<Paid, AskError> {
        self.ask_at(&format!("{}/engine/v1/ask", self.node), intent, params, budget).await
    }

    /// Direct ask to one miner (no fallback inside the node).
    pub async fn ask_direct(
        &self,
        miner: &str,
        intent: &str,
        params: Value,
        budget: impl FnOnce(u64) -> Result<(), String>,
    ) -> Result<Paid, AskError> {
        self.ask_at(&format!("{}/engine/v1/ask/{miner}", self.node), intent, params, budget).await
    }

    async fn post(&self, url: &str, body: &Value, payment: Option<&str>) -> Result<reqwest::Response, AskError> {
        let mut req = self.http.post(url).json(body);
        if let Some(p) = payment {
            req = req.header("X-PAYMENT", p);
        }
        req.send().await.map_err(|e| AskError::Unreachable(e.to_string()))
    }

    async fn ask_at(&self, url: &str, intent: &str, params: Value, budget: impl FnOnce(u64) -> Result<(), String>) -> Result<Paid, AskError> {
        let body = json!({ "intent": intent, "params": params });

        // 1. unpaid request -> challenge
        let res = self.post(url, &body, None).await?;
        let status = res.status().as_u16();
        let text = res.text().await.map_err(|e| AskError::Unreachable(e.to_string()))?;
        match status {
            402 => {}
            200 => {
                return Ok(Paid { body: serde_json::from_str(&text).unwrap_or(Value::Null), receipt: Value::Null, amount_uusdc: 0 });
            }
            400 => return Err(AskError::BadRequest(reason_of(status, &text))),
            503 => return Err(AskError::NoCompatibleMiner(reason_of(status, &text))),
            _ => return Err(AskError::Refused { status, reason: reason_of(status, &text) }),
        }
        let challenge_body: Value = serde_json::from_str(&text).map_err(|_| AskError::UnexpectedChallenge("402 body is not JSON".into()))?;
        let ch = parse_challenge(&challenge_body, &self.diamond, &self.network)?;

        // 2. guards before signing
        let signer = self.signer.as_ref().ok_or(AskError::NoWallet)?;
        if ch.amount > self.max_price {
            return Err(AskError::PriceTooHigh { price: ch.amount, max: self.max_price });
        }
        budget(ch.amount).map_err(|reason| AskError::BudgetDenied { price: ch.amount, reason })?;

        // 3. sign
        let from = format!("{:#x}", signer.address());
        let now = chrono::Utc::now().timestamp();
        let (va, vb) = (now - 5, now + 100);
        let nonce = uuid::Uuid::new_v4().to_string();
        let message = ch
            .template
            .replace("{from}", &from)
            .replace("{nonce}", &nonce)
            .replace("{validAfter}", &va.to_string())
            .replace("{validBefore}", &vb.to_string());
        let sig = signer.sign_message_sync(message.as_bytes()).map_err(|e| AskError::UnexpectedChallenge(format!("signing failed: {e}")))?;
        let payment = json!({
            "x402Version": 2,
            "scheme": "escrow",
            "network": self.network,
            "payload": {
                "signature": format!("0x{}", hex::encode(sig.as_bytes())),
                "authorization": {
                    "from": from, "to": ch.pay_to, "value": ch.amount.to_string(),
                    "validAfter": va.to_string(), "validBefore": vb.to_string(), "nonce": nonce,
                }
            }
        });
        let header = B64.encode(payment.to_string());

        // 4. paid request
        let res = self.post(url, &body, Some(&header)).await?;
        let status = res.status().as_u16();
        let receipt = res
            .headers()
            .get("x-payment-response")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| B64.decode(h.trim()).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .unwrap_or(Value::Null);
        let text = res.text().await.map_err(|e| AskError::Unreachable(e.to_string()))?;
        if status != 200 {
            return Err(AskError::Refused { status, reason: reason_of(status, &text) });
        }
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let paid = Paid { body, receipt, amount_uusdc: ch.amount };
        tracing::info!(
            target: "purchase",
            endpoint = url, intent, amount_uusdc = ch.amount, miner_id = %paid.miner_id(), miner_name = %paid.miner_name(),
            receipt_hash = %paid.receipt_hash(), signal_hash = %paid.signal_hash(), "telegraph purchase"
        );
        Ok(paid)
    }
}

#[cfg(test)]
pub mod fake {
    //! In-process fake Telegraph node: issues escrow challenges, verifies EIP-191 signatures,
    //! and answers routed and direct asks with configurable bodies.
    use super::*;
    use alloy_primitives::{Address, Signature};
    use axum::{
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };
    use std::sync::{Arc, Mutex};

    pub const DIAMOND: &str = "0x72a70e949362670Bf8bceAd24eFC5b8f950aa08b";
    /// Well-known test key (anvil #0). Never holds real funds.
    pub const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    #[derive(Default)]
    pub struct FakeState {
        pub routed: Option<Value>,
        pub direct: Option<Value>,
        pub price: u64,
        pub pay_to: Option<String>,
        pub paid_calls: Vec<(String, Address, u64)>,
        pub challenges: usize,
    }

    pub type Shared = Arc<Mutex<FakeState>>;

    fn challenge(st: &FakeState, resource: &str) -> Value {
        let to = st.pay_to.clone().unwrap_or(DIAMOND.to_lowercase());
        json!({ "accepts": [
            { "scheme": "exact", "network": "eip155:11155111", "payTo": "0x0000000000000000000000000000000000000001" },
            { "scheme": "escrow", "network": "eip155:11155111", "payTo": to, "extra": { "message": format!(
                "Telegraph Escrow Payment\n\nWallet: {{from}}\nPay To: {to}\nNetwork: eip155:11155111\nAmount: {} uUSDC\nResource: {resource}\nNonce: {{nonce}}\nValid After: {{validAfter}}\nValid Before: {{validBefore}}",
                st.price) } }
        ]})
    }

    async fn handle(st: Shared, resource: String, headers: HeaderMap, body: Option<Value>) -> axum::response::Response {
        let has_answer = {
            let g = st.lock().unwrap();
            if resource.contains("/ask/") { g.direct.is_some() } else { g.routed.is_some() }
        };
        if !has_answer {
            // like the real node: 503 before any payment
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "no_compatible_miner"}))).into_response();
        }
        let Some(p) = headers.get("x-payment") else {
            let mut s = st.lock().unwrap();
            s.challenges += 1;
            return (StatusCode::PAYMENT_REQUIRED, Json(challenge(&s, &resource))).into_response();
        };
        let pay: Value = serde_json::from_slice(&B64.decode(p.as_bytes()).unwrap()).unwrap();
        let auth = &pay["payload"]["authorization"];
        let s = |k: &str| auth[k].as_str().unwrap().to_string();
        let to = st.lock().unwrap().pay_to.clone().unwrap_or(DIAMOND.to_lowercase());
        let price = st.lock().unwrap().price;
        // Rebuild the exact message the client must have signed, then recover the signer.
        let msg = format!(
            "Telegraph Escrow Payment\n\nWallet: {}\nPay To: {to}\nNetwork: eip155:11155111\nAmount: {price} uUSDC\nResource: {resource}\nNonce: {}\nValid After: {}\nValid Before: {}",
            s("from"), s("nonce"), s("validAfter"), s("validBefore")
        );
        let sig: Signature = pay["payload"]["signature"].as_str().unwrap().parse().unwrap();
        let signer = sig.recover_address_from_msg(msg.as_bytes()).unwrap();
        let window = s("validBefore").parse::<i64>().unwrap() - s("validAfter").parse::<i64>().unwrap();
        if format!("{signer:#x}") != s("from") || window > 120 || s("value") != price.to_string() {
            return (StatusCode::PAYMENT_REQUIRED, Json(json!({"reason": "bad signature"}))).into_response();
        }
        let answer = {
            let mut g = st.lock().unwrap();
            g.paid_calls.push((resource.clone(), signer, price));
            if resource.contains("/ask/") { g.direct.clone() } else { g.routed.clone() }
        };
        let _ = body;
        let receipt = json!({ "receipt_hash": format!("0xreceipt{}", st.lock().unwrap().paid_calls.len()), "rail": "escrow",
                              "payer": signer, "amount_uusdc": price.to_string() });
        match answer {
            Some(a) => (StatusCode::OK, [("x-payment-response", B64.encode(receipt.to_string()))], Json(a)).into_response(),
            None => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "no_compatible_miner"}))).into_response(),
        }
    }

    pub async fn spawn(state: FakeState) -> (String, Shared) {
        let shared: Shared = Arc::new(Mutex::new(state));
        let app = Router::new()
            .route(
                "/engine/v1/ask",
                post(|State(st): State<Shared>, h: HeaderMap, b: Json<Value>| async move { handle(st, "/v1/ask".into(), h, Some(b.0)).await }),
            )
            .route(
                "/engine/v1/ask/{miner}",
                post(|State(st): State<Shared>, Path(m): Path<String>, h: HeaderMap, b: Json<Value>| async move {
                    handle(st, format!("/v1/ask/{m}"), h, Some(b.0)).await
                }),
            )
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), shared)
    }

    pub fn client(node: &str) -> Client {
        Client::new(node, DIAMOND, "eip155:11155111", Some(KEY), Duration::from_secs(5), 50_000).unwrap()
    }

    pub fn sla_answer(service: &str, uptime: f64, target: f64) -> Value {
        json!({
            "miner_id": "7410311", "miner_name": "Statuspage SLA Auditor", "result_shape": "intent",
            "result": { "service": service, "uptime_pct": uptime, "sla_target_pct": target, "compliant": uptime >= target, "period": "last 30 days" },
            "field_status": { "service": "ok", "uptime_pct": "ok", "sla_target_pct": "ok", "compliant": "ok", "period": "ok" },
            "intent": "SLA_COMPLIANCE_GENERAL", "cost_usd": 0.01, "signal_hash": "0xsignal"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    fn params() -> Value {
        json!({"service": "GitHub", "period": "last 30 days"})
    }

    #[tokio::test]
    async fn pays_with_valid_eip191_signature() {
        let (node, st) = spawn(FakeState { routed: Some(sla_answer("GitHub", 99.2, 99.9)), price: 10_000, ..Default::default() }).await;
        let c = client(&node);
        let paid = c.ask("SLA_COMPLIANCE_GENERAL", params(), |_| Ok(())).await.unwrap();
        assert_eq!(paid.amount_uusdc, 10_000);
        assert_eq!(paid.receipt_hash(), "0xreceipt1");
        assert_eq!(paid.signal_hash(), "0xsignal");
        let calls = &st.lock().unwrap().paid_calls;
        assert_eq!(calls.len(), 1);
        assert_eq!(format!("{:#x}", calls[0].1), c.address().unwrap());
    }

    #[tokio::test]
    async fn direct_ask_hits_miner_path() {
        let (node, st) = spawn(FakeState { direct: Some(sla_answer("GitHub", 99.2, 99.9)), price: 10_000, ..Default::default() }).await;
        client(&node).ask_direct("7410311", "SLA_COMPLIANCE_GENERAL", params(), |_| Ok(())).await.unwrap();
        assert_eq!(st.lock().unwrap().paid_calls[0].0, "/v1/ask/7410311");
    }

    #[tokio::test]
    async fn zero_budget_refuses_before_signing() {
        let (node, st) = spawn(FakeState { routed: Some(sla_answer("GitHub", 99.2, 99.9)), price: 10_000, ..Default::default() }).await;
        let err = client(&node).ask("SLA_COMPLIANCE_GENERAL", params(), |_| Err("per-incident budget is 0".into())).await.unwrap_err();
        assert!(matches!(err, AskError::BudgetDenied { price: 10_000, .. }), "{err}");
        let s = st.lock().unwrap();
        assert_eq!((s.challenges, s.paid_calls.len()), (1, 0));
    }

    #[tokio::test]
    async fn refuses_foreign_pay_to() {
        let (node, st) = spawn(FakeState {
            routed: Some(sla_answer("GitHub", 99.2, 99.9)),
            price: 10_000,
            pay_to: Some("0x000000000000000000000000000000000000dead".into()),
            ..Default::default()
        })
        .await;
        let err = client(&node).ask("SLA_COMPLIANCE_GENERAL", params(), |_| Ok(())).await.unwrap_err();
        assert!(matches!(err, AskError::UnexpectedChallenge(_)), "{err}");
        assert!(st.lock().unwrap().paid_calls.is_empty());
    }

    #[tokio::test]
    async fn price_cap() {
        let (node, _) = spawn(FakeState { routed: Some(json!({})), price: 60_000, ..Default::default() }).await;
        let err = client(&node).ask("X", params(), |_| Ok(())).await.unwrap_err();
        assert!(matches!(err, AskError::PriceTooHigh { price: 60_000, max: 50_000 }));
    }

    #[tokio::test]
    async fn unreachable_node() {
        let c = client("http://127.0.0.1:9"); // discard port: connection refused
        let err = c.ask("SLA_COMPLIANCE_GENERAL", params(), |_| Ok(())).await.unwrap_err();
        assert!(matches!(err, AskError::Unreachable(_)), "{err}");
        assert!(!err.retryable());
    }

    #[tokio::test]
    async fn no_wallet() {
        let (node, _) = spawn(FakeState { routed: Some(json!({})), price: 10_000, ..Default::default() }).await;
        let c = Client::new(&node, DIAMOND, "eip155:11155111", None, Duration::from_secs(5), 50_000).unwrap();
        assert!(matches!(c.ask("X", params(), |_| Ok(())).await.unwrap_err(), AskError::NoWallet));
    }

    #[test]
    fn challenge_amount_parse() {
        let body = json!({"accepts":[{"scheme":"escrow","network":"eip155:11155111","payTo":DIAMOND,
            "extra":{"message":"X\nAmount: 12345 uUSDC\n{from}{nonce}{validAfter}{validBefore}"}}]});
        assert_eq!(parse_challenge(&body, DIAMOND, "eip155:11155111").unwrap().amount, 12345);
    }
}
