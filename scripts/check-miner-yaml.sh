#!/usr/bin/env bash
# Offline/free checks for a miner YAML against the documented closed schema
# (https://anjoman.ai/docs/miners/yaml-config). The authoritative validator is the
# dashboard at https://anjoman.ai/dashboard/earn (requires login).
#   scripts/check-miner-yaml.sh sla-miner/miner.yaml
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/.env"
Y="${1:-$ROOT/sla-miner/miner.yaml}"
J=$(npx --yes js-yaml "$Y")
fail=0
ok()  { echo "  ok   $*"; }
bad() { echo "  FAIL $*"; fail=1; }

echo "YAML parses"; ok "$Y"

chk_keys() { # $1 label, $2 jq path to object(s), $3 allowed keys (space separated)
  local extra
  extra=$(jq -r --arg a "$3" "[$2 | keys[]] - (\$a|split(\" \")) | unique | join(\",\")" <<<"$J")
  [[ -z "$extra" ]] && ok "$1 keys" || bad "$1 unknown keys: $extra"
}
chk_keys root '.' "version kind id slug protocol name description base_url input_schema output_schema intent_mapping polling cache_ttl_sec rate_limit_per_sec circuit_threshold circuit_cooldown_seconds auth endpoints semantics docs limitations errors on_chain"
for k in version kind id slug name base_url; do
  [[ "$(jq "has(\"$k\")" <<<"$J")" == true ]] && ok "required $k" || bad "missing $k"
done
chk_keys endpoint '.endpoints[]' "path external_path method description intents params endpoint_base_url content_type multipart_fields param_map"
chk_keys param '.endpoints[].params.query[]?[]' "name type intents description accepted_fields"
chk_keys semantics '.semantics' "signal_mapping supported_intents"
chk_keys limitation '.limitations[]?' "code message param property value_bytes value_num operator window_seconds"
chk_keys errors '.errors // {}' "message_path code_path status_path success_values"
chk_keys docs '.docs // {}' "website documentation repository twitter discord"
chk_keys mapping.input '.intent_mapping[].input[]' "from how add_hours_from default_hours time_format value"
chk_keys mapping.output '.intent_mapping[].output[]' "path unit reduce formula how"

[[ "$(jq -r .version <<<"$J")" == "1" ]] && ok 'version "1"' || bad 'version must be "1"'
jq -e '.slug|test("^[a-z0-9]+(-[a-z0-9]+)*$")' <<<"$J" >/dev/null && ok "slug kebab-case" || bad "slug pattern"
jq -e '.base_url|startswith("https://")' <<<"$J" >/dev/null && ok "base_url https" || bad "base_url must be https"

# every intent_mapping endpoint is declared; every output field of the intent is mapped
for ep in $(jq -r '.intent_mapping[].endpoint' <<<"$J"); do
  jq -e --arg e "$ep" '[.endpoints[].path]|index($e)' <<<"$J" >/dev/null && ok "mapping endpoint $ep declared" || bad "mapping endpoint $ep not declared"
done
for intent in $(jq -r '.semantics.supported_intents[]' <<<"$J"); do
  c=$(cast call "$DIAMOND" 'isCanonicalIntent(string)(bool)' "$intent" --rpc-url "$RPC")
  [[ "$c" == true ]] && ok "$intent canonical" || bad "$intent not canonical"
  want=$(curl -s "$NODE/engine/v1/intents/$intent" | jq -r '.response.fields[].name' 2>/dev/null || true)
  for f in $want; do
    jq -e --arg i "$intent" --arg f "$f" '.intent_mapping[$i].output[$f].path' <<<"$J" >/dev/null && ok "$intent output.$f mapped" || bad "$intent output.$f not mapped"
  done
done

# id and slug unused by anyone else
ID=$(jq -r .id <<<"$J"); SLUG=$(jq -r .slug <<<"$J")
taken=$(curl -s "$NODE/api/miners" | jq --arg id "$ID" --arg s "$SLUG" --arg w "$(tr A-Z a-z <<<"$ADDR")" \
  '[.[] | select(((.id|tostring)==$id or .slug==$s) and ((.wallet_address//"")|ascii_downcase) != $w)] | length')
[[ "$taken" == 0 ]] && ok "id $ID and slug $SLUG free" || bad "id/slug taken by another wallet"

# live endpoint answers with every mapped field
BASE=$(jq -r .base_url <<<"$J")
if R=$(curl -sf -m 20 "$BASE/sla?service=GitHub&period=last%2030%20days"); then
  for f in $(jq -r '.intent_mapping[].output | to_entries[] | .value.path' <<<"$J"); do
    jq -e --arg f "$f" 'has($f)' <<<"$R" >/dev/null && ok "live $BASE has $f" || bad "live response missing $f"
  done
else
  bad "base_url $BASE not reachable (deploy first)"
fi
exit $fail
