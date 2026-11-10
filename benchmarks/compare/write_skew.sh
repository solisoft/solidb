#!/bin/bash
# Write skew on SoliDB 2.9.3: two doctors on call; each transaction takes one
# off call if both are on. Run against a throwaway server.
#   B=http://127.0.0.1:6811 PASS=bench ./skew.sh
set -u
B=${B:-http://127.0.0.1:6811}
PASS=${PASS:-bench}
TOKEN=$(curl -s -X POST $B/auth/login -H 'content-type: application/json' \
  -d "{\"database\":\"_system\",\"username\":\"admin\",\"password\":\"$PASS\"}" |
  python3 -c 'import json,sys;print(json.load(sys.stdin)["token"])')
H=(-H "authorization: Bearer $TOKEN" -H 'content-type: application/json')
D=$B/_api/database/skew
c() { echo "\$ $1 $2"; curl -s -X "$1" "${H[@]}" "$2" ${3:+-d "$3"}; echo; }
tx() { curl -s -X POST "${H[@]}" "$D/transaction/begin" -d '{"isolationLevel":"serializable"}' |
  python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])'; }
reset() {
  curl -s -X POST "${H[@]}" "$D/cursor" -d '{"query":"FOR d IN doctors REMOVE d IN doctors"}' >/dev/null
  curl -s -X POST "${H[@]}" "$D/cursor" -d '{"query":"FOR k IN [\"alice\",\"bob\"] INSERT {_key: k, on_call: true} INTO doctors"}' >/dev/null
}
oncall() { c POST "$D/cursor" '{"query":"FOR d IN doctors SORT d._key RETURN [d._key, d.on_call]"}'; }

curl -s -X POST "${H[@]}" $B/_api/database -d '{"name":"skew"}' >/dev/null
curl -s -X POST "${H[@]}" $D/collection -d '{"name":"doctors"}' >/dev/null

# Take @me off call, but only if at least two doctors are on call.
Q='FOR d IN doctors FILTER d._key == @me
  LET n = LENGTH(FOR x IN doctors FILTER x.on_call == true RETURN 1)
  FILTER n >= 2
  UPDATE d WITH {on_call: false} IN doctors RETURN n'

echo "== queries"
reset
T1=$(tx); T2=$(tx)
c POST "$D/transaction/$T1/query" "$(python3 -c 'import json,sys;print(json.dumps({"query":sys.argv[1],"bindVars":{"me":"alice"}}))' "$Q")"
c POST "$D/transaction/$T2/query" "$(python3 -c 'import json,sys;print(json.dumps({"query":sys.argv[1],"bindVars":{"me":"bob"}}))' "$Q")"
c POST "$D/transaction/$T1/commit"
c POST "$D/transaction/$T2/commit"
oncall

echo "== documents by key"
reset
T1=$(tx); T2=$(tx)
c GET "$D/transaction/$T1/document/doctors/alice"
c GET "$D/transaction/$T1/document/doctors/bob"
c GET "$D/transaction/$T2/document/doctors/alice"
c GET "$D/transaction/$T2/document/doctors/bob"
c PUT "$D/transaction/$T1/document/doctors/alice" '{"on_call": false}'
c POST "$D/transaction/$T1/rollback"
c PUT "$D/transaction/$T2/document/doctors/bob" '{"on_call": false}'
c POST "$D/transaction/$T2/commit"
oncall
