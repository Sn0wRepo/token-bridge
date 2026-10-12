//! Blocking JSON-RPC client with the handful of methods the relayer needs.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::abi::RawLog;
use crate::types::{from_hex, quantity, to_hex, Address, H256};

pub struct Rpc {
    url: String,
    agent: ureq::Agent,
}

impl Rpc {
    pub fn new(url: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build();
        Rpc {
            url: url.to_string(),
            agent,
        }
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let resp: Value = self
            .agent
            .post(&self.url)
            .send_json(body)
            .with_context(|| format!("{method} request failed"))?
            .into_json()
            .with_context(|| format!("{method}: invalid JSON response"))?;
        if let Some(err) = resp.get("error") {
            bail!("{method}: {err}");
        }
        resp.get("result")
            .cloned()
            .ok_or_else(|| anyhow!("{method}: missing result"))
    }

    fn quantity(&self, method: &str, params: Value) -> Result<u128> {
        let v = self.call(method, params)?;
        quantity(
            v.as_str()
                .ok_or_else(|| anyhow!("{method}: expected string"))?,
        )
    }

    pub fn chain_id(&self) -> Result<u64> {
        Ok(self.quantity("eth_chainId", json!([]))? as u64)
    }

    pub fn block_number(&self) -> Result<u64> {
        Ok(self.quantity("eth_blockNumber", json!([]))? as u64)
    }

    pub fn nonce(&self, who: &Address) -> Result<u64> {
        Ok(self.quantity(
            "eth_getTransactionCount",
            json!([who.to_string(), "pending"]),
        )? as u64)
    }

    pub fn gas_price(&self) -> Result<u128> {
        self.quantity("eth_gasPrice", json!([]))
    }

    pub fn max_priority_fee(&self) -> Result<u128> {
        self.quantity("eth_maxPriorityFeePerGas", json!([]))
    }

    pub fn estimate_gas(&self, from: &Address, to: &Address, data: &[u8]) -> Result<u64> {
        let tx = json!({"from": from.to_string(), "to": to.to_string(), "data": to_hex(data)});
        Ok(self.quantity("eth_estimateGas", json!([tx]))? as u64)
    }

    pub fn eth_call(&self, to: &Address, data: &[u8]) -> Result<Vec<u8>> {
        let v = self.call(
            "eth_call",
            json!([{"to": to.to_string(), "data": to_hex(data)}, "latest"]),
        )?;
        from_hex(
            v.as_str()
                .ok_or_else(|| anyhow!("eth_call: expected string"))?,
        )
    }

    pub fn logs(
        &self,
        address: &Address,
        topic0: &H256,
        from: u64,
        to: u64,
    ) -> Result<Vec<RawLog>> {
        let filter = json!({
            "address": address.to_string(),
            "topics": [topic0.to_string()],
            "fromBlock": format!("0x{from:x}"),
            "toBlock": format!("0x{to:x}"),
        });
        let v = self.call("eth_getLogs", json!([filter]))?;
        serde_json::from_value(v).context("eth_getLogs: unexpected shape")
    }

    pub fn send_raw(&self, raw: &[u8]) -> Result<H256> {
        let v = self.call("eth_sendRawTransaction", json!([to_hex(raw)]))?;
        v.as_str()
            .ok_or_else(|| anyhow!("expected tx hash"))?
            .parse()
    }

    /// Polls until the receipt exists; returns `true` when the transaction succeeded.
    pub fn wait_receipt(&self, hash: &H256, timeout: Duration) -> Result<bool> {
        let start = std::time::Instant::now();
        loop {
            let r = self.call("eth_getTransactionReceipt", json!([hash.to_string()]))?;
            if !r.is_null() {
                return Ok(r.get("status").and_then(Value::as_str) == Some("0x1"));
            }
            if start.elapsed() > timeout {
                bail!("timed out waiting for {hash}");
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}
