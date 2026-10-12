#!/usr/bin/env bash
# End-to-end test: two local chains (Anvil as "Ethereum" id 1 and "Base" id 8453), real vault
# deployments, a real bridgeNative transaction, and the Rust relayer releasing on the other side.
#
# Requires: foundry (anvil, forge, cast), cargo. Run from the repo root: ./scripts/e2e.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"
# Native binaries on Windows (Git Bash) need a Windows-style path.
command -v cygpath >/dev/null 2>&1 && WORK="$(cygpath -m "$WORK")"
L1=http://127.0.0.1:18545
L2=http://127.0.0.1:18546
SOLANA_ID=1399811149

# Anvil's well-known dev keys. Never use them anywhere else.
DEPLOYER_PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
RELAYER_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
RELAYER=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
USER_PK=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a
RECIPIENT=0x90F79bf6EB2c4f870365E785982E1f101E93b906
FEE_TO=0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65

cleanup() {
  kill "${PID1:-}" "${PID2:-}" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

step() { printf '\n==> %s\n' "$*"; }

step "Starting two Anvil chains"
anvil --port 18545 --chain-id 1 --silent &
PID1=$!
anvil --port 18546 --chain-id 8453 --silent &
PID2=$!
for url in $L1 $L2; do
  for _ in $(seq 1 50); do cast chain-id --rpc-url "$url" >/dev/null 2>&1 && break; sleep 0.2; done
done

deploy_vault() {
  forge create src/BridgeVault.sol:BridgeVault --root "$ROOT" --rpc-url "$1" --private-key $DEPLOYER_PK \
    --broadcast --constructor-args $RELAYER $FEE_TO 10 \
    | grep -oE '(Deployed to: |"deployedTo":")0x[0-9a-fA-F]{40}' | grep -oE '0x[0-9a-fA-F]{40}'
}

step "Deploying BridgeVault on both chains"
V1=$(deploy_vault $L1)
V2=$(deploy_vault $L2)
echo "ethereum vault $V1"
echo "base vault     $V2"
cast send "$V1" "setDestination(uint256,bool)" 8453 true --rpc-url $L1 --private-key $DEPLOYER_PK >/dev/null
cast send "$V1" "setDestination(uint256,bool)" $SOLANA_ID true --rpc-url $L1 --private-key $DEPLOYER_PK >/dev/null
cast send "$V2" --value 10ether --rpc-url $L2 --private-key $DEPLOYER_PK >/dev/null   # release liquidity

step "User bridges 1 ETH Ethereum -> Base and 0.5 ETH Ethereum -> Solana"
cast send "$V1" "bridgeNative(uint256,bytes32)" 8453 "$(cast to-uint256 $RECIPIENT)" \
  --value 1ether --rpc-url $L1 --private-key $USER_PK >/dev/null
SOL_KEY=0x7f2f8f1f5a7b1d0e3bf3c3f8f1d9e5b2a0c4d6e8f0a1b2c3d4e5f60718293a4b
cast send "$V1" "bridgeNative(uint256,bytes32)" $SOLANA_ID $SOL_KEY \
  --value 0.5ether --rpc-url $L1 --private-key $USER_PK >/dev/null

cat >"$WORK/relayer.toml" <<EOF
state_file = "$WORK/state.json"
poll_interval_secs = 1

[[chains]]
name = "ethereum"
chain_id = 1
rpc_url = "$L1"
vault = "$V1"
confirmations = 0

[[chains]]
name = "base"
chain_id = 8453
rpc_url = "$L2"
vault = "$V2"
confirmations = 0

[solana]
chain_id = $SOLANA_ID
outbox = "$WORK/solana-outbox.jsonl"
EOF

BEFORE=$(cast balance $RECIPIENT --rpc-url $L2)

step "Relayer dry-run (must not send anything)"
cargo run -q --manifest-path "$ROOT/relayer/Cargo.toml" -- -c "$WORK/relayer.toml" run --once
[ ! -e "$WORK/solana-outbox.jsonl" ] || { echo "dry-run wrote the Solana outbox"; exit 1; }
rm -f "$WORK/state.json"
[ "$(cast balance $RECIPIENT --rpc-url $L2)" = "$BEFORE" ] || { echo "dry-run moved funds"; exit 1; }

step "Relayer --execute"
RELAYER_PRIVATE_KEY=$RELAYER_PK cargo run -q --manifest-path "$ROOT/relayer/Cargo.toml" -- \
  -c "$WORK/relayer.toml" run --once --execute

AFTER=$(cast balance $RECIPIENT --rpc-url $L2)
GOT=$(python3 -c "print($AFTER - $BEFORE)" 2>/dev/null || python -c "print($AFTER - $BEFORE)")
echo "recipient received $GOT wei on Base"
[ "$GOT" = "999000000000000000" ] || { echo "expected 0.999 ETH"; exit 1; }

PROCESSED=$(cast call "$V2" "processed(bytes32)(bool)" "$(cast call "$V2" "transferId(uint256,uint256)(bytes32)" 1 1 --rpc-url $L2)" --rpc-url $L2)
[ "$PROCESSED" = "true" ] || { echo "transfer not marked processed"; exit 1; }

grep -q '"recipient":"9ZUqfjX2A9HwbcngRieoV4b7ECNHS7b4DCtJj2fQ99GN"' "$WORK/solana-outbox.jsonl" \
  || { echo "solana job missing"; cat "$WORK/solana-outbox.jsonl"; exit 1; }
echo "solana job: $(cat "$WORK/solana-outbox.jsonl")"

step "Second run from scratch is idempotent (processed check)"
rm -f "$WORK/state.json"
RELAYER_PRIVATE_KEY=$RELAYER_PK cargo run -q --manifest-path "$ROOT/relayer/Cargo.toml" -- \
  -c "$WORK/relayer.toml" run --once --execute 2>&1 | grep -q "already processed"
[ "$(cast balance $RECIPIENT --rpc-url $L2)" = "$AFTER" ] || { echo "double release"; exit 1; }

printf '\nE2E OK\n'
