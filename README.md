# token-bridge

[![CI](https://github.com/Sn0wRepo/token-bridge/actions/workflows/ci.yml/badge.svg)](https://github.com/Sn0wRepo/token-bridge/actions/workflows/ci.yml)
![Solidity](https://img.shields.io/badge/Solidity-0.8.26-363636?logo=solidity)
![Rust](https://img.shields.io/badge/Rust-stable-orange?logo=rust)
![License](https://img.shields.io/badge/license-MIT-blue)

A lock-and-release / burn-and-mint bridge for **Ethereum ⇄ Base** and **Ethereum/Base → Solana**. It ships with vault contracts, a reference relayer written in Rust, and a single-file web UI.

> **Reference implementation, not audited.** Releases are authorized by a single relayer key. See [Roadmap](#roadmap) before any real funds touch it.

| Part | Path | What it does |
| --- | --- | --- |
| Contracts | `src/BridgeVault.sol`, `src/BridgedToken.sol` | Lock or burn on the source chain, release or mint on the destination. Includes replay protection, a fee capped at 1%, pause, and a reentrancy guard |
| Relayer (Rust) | `relayer/` | Watches `BridgeRequested`, waits for confirmations, checks `processed()`, then signs and sends EIP-1559 `release()` transactions. Solana jobs go to a JSONL outbox |
| Web app | `web/index.html` | Bridge UI with MetaMask and Phantom support. Runs in demo mode until vault addresses are configured |

## How it works

```
Source chain                         Relayer (Rust)                  Destination chain
user -> bridgeNative / bridgeERC20   eth_getLogs (finalized range)   BridgeVault.release()
emits BridgeRequested(nonce, ...) -> decode, check processed()  ->    native / locked ERC-20 / mint wrapped
                                     sign EIP-1559, send             Solana: JSONL outbox -> executor
```

- Recipients are `bytes32`: a left-padded EVM address, or a raw 32-byte Solana public key.
- Replay protection uses `keccak256(abi.encode(srcChainId, srcNonce))`, both on-chain and in the relayer.
- Scan progress is persisted atomically. A failed range is retried, and retries are idempotent.

## Quick start

```bash
forge test                                   # 19 contract tests
cd relayer && cargo test                     # unit tests + signing vectors pinned against ethers.js
./scripts/e2e.sh                             # 2 local chains, real deploys, real bridge + relay
```

Running the relayer:

```bash
cp relayer/relayer.example.toml relayer.toml          # fill in RPCs and vaults
cargo run --release --manifest-path relayer/Cargo.toml -- check
cargo run --release --manifest-path relayer/Cargo.toml -- run            # dry-run
RELAYER_PRIVATE_KEY=0x... cargo run --release --manifest-path relayer/Cargo.toml -- run --execute
```

Deploying a vault:

```bash
RELAYER=0x.. FEE_RECIPIENT=0x.. FEE_BPS=10 DESTINATIONS=8453,1399811149 \
  forge script script/Deploy.s.sol --rpc-url $RPC_URL --account deployer --broadcast
```

## Tests

- **Foundry (19):** native and ERC-20 (including USDT-style tokens that return no bool), wrapped round trip, replay, relayer-only access, pause, fee cap, reentrant fee sink, and fuzzed fee math.
- **Rust (12):** RLP spec vectors, ABI decoding, and signed EIP-1559 transactions identical to ethers.js output.
- **E2E:** two Anvil chains. The recipient gets exactly the net amount on the destination, the Solana job is queued once, and a full rescan doesn't release twice.

## Roadmap

- [ ] Ethereum ⇄ Base through the canonical OP Stack bridge
- [ ] Solana through Wormhole NTT, LayerZero OFT or CCIP, plus an Anchor program for the inbound side
- [ ] M-of-N attestation instead of a single relayer key
- [ ] Rate limits and per-transfer caps
- [ ] External audit

## License

MIT
