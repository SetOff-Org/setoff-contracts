#!/usr/bin/env bash
# Deploys the settlement contract to testnet and runs one netting cycle:
# three members owe each other 10 XLM in a ring, only A holds collateral,
# and settlement moves nothing because every net position is zero.
#
# Needs stellar-cli 25.2 or later. Prints every transaction hash.
set -euo pipefail
cd "$(dirname "$0")/.."

NETWORK=${NETWORK:-testnet}
WASM=target/wasm32v1-none/release/setoff_settlement.wasm
XLM_10=100000000 # 10 XLM in stroops

stellar contract build >/dev/null

for who in setoff-operator setoff-a setoff-b setoff-c; do
  stellar keys generate "$who" --network "$NETWORK" --fund --overwrite >/dev/null 2>&1
done
OP=$(stellar keys address setoff-operator)
A=$(stellar keys address setoff-a)
B=$(stellar keys address setoff-b)
C=$(stellar keys address setoff-c)
XLM=$(stellar contract id asset --asset native --network "$NETWORK")

# Runs a contract call, printing a label and the transaction hash.
step() {
  local label=$1 source=$2
  shift 2
  local out
  out=$(stellar contract invoke --id "$ID" --source "$source" --network "$NETWORK" --send yes -- "$@" 2>&1)
  local hash
  hash=$(grep -oE '[0-9a-f]{64}' <<<"$out" | head -1 || true)
  printf '%-58s %s\n' "$label" "${hash:-(read-only)}"
}

ID=$(stellar contract deploy --wasm "$WASM" --source setoff-operator --network "$NETWORK" -- --admin "$OP" 2>/dev/null)
echo "contract $ID"

ref() { printf '%064x' "$1"; }
owe() { printf '[{"debtor":"%s","creditor":"%s","token":"%s","amount":"%s","reference":"%s"}]' "$1" "$2" "$XLM" "$XLM_10" "$(ref "$3")"; }

step "allow native XLM" setoff-operator set_token --token "$XLM" --allowed true
step "admit A, B and C in one call" setoff-operator admit_many --members "[\"$A\",\"$B\",\"$C\"]"
step "obligations below 1 XLM are refused" setoff-operator set_min_amount --token "$XLM" --amount 10000000
step "A deposits 10 XLM collateral" setoff-a deposit --member "$A" --token "$XLM" --amount "$XLM_10"
step "A owes B 10 XLM" setoff-a submit --obligations "$(owe "$A" "$B" 1)"
step "B owes C 10 XLM with no collateral: B is owed 10" setoff-b submit --obligations "$(owe "$B" "$C" 1)"
step "C owes A 10 XLM" setoff-c submit --obligations "$(owe "$C" "$A" 1)"
step "settle: 30 XLM gross, every position 0" setoff-operator settle
step "A takes its collateral back" setoff-a withdraw_all --member "$A" --token "$XLM"
