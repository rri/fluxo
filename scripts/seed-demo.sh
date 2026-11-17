#!/bin/sh
# Enter a sample goal ("Launch fluxo.dev beta") into a fresh fluxo through its own forms,
# exactly as a person would. Usage: scripts/seed-demo.sh http://127.0.0.1:8080
# The server must be brand new (it runs first-time setup as "ada"); prints the session cookie jar path.
set -eu
B=${1:-http://127.0.0.1:8080}
J=$(mktemp)
token() { curl -s -c "$J" -b "$J" "$B$1" | grep -o 'name="csrf" value="[^"]*"' | head -1 | cut -d'"' -f4; }
post() { curl -s -o /dev/null -w '%{redirect_url}' -c "$J" -b "$J" --data "$2" "$B$1"; }
id_of() { sed 's|.*/||; s|#.*||; s|?.*||'; }

T=$(token "/setup?token=${SETUP_TOKEN:?set SETUP_TOKEN to the server's setup token}")
post /setup "csrf=$T&setup_token=$SETUP_TOKEN&username=ada&password=a+long+enough+password&confirm=a+long+enough+password" >/dev/null
T=$(token /goals/new)
G=$(post /goals "csrf=$T&title=Launch+fluxo.dev+beta&why=Invite+the+first+50+teams+to+plan+real+goals+on+fluxo.dev.&parent=" | id_of)

step() { # title kind estimate-or-coverage clarity needs
  if [ "$2" = goal ]; then extra="coverage=$3"; else extra="estimate=$3&clarity=$4"; fi
  post "/goals/$G/steps" "csrf=$T&title=$1&kind=$2&$extra&needs=$5" >/dev/null
  curl -s -b "$J" "$B/goals/$G" | grep -o 'data-id="[0-9]*"' | tail -1 | grep -o '[0-9]*'
}
SCHEMA=$(step SQLite+schema+v1 task 3 high "")
API=$(step Rust+API+skeleton task 5 medium "$SCHEMA")
ENGINE=$(step Simulation+engine task 12 medium "$SCHEMA")
AUTH=$(step Accounts+%26+auth goal partial "" "$API")
PATHUI=$(step Path+view+UI task 6 medium "$ENGINE")
post "/nodes/$PATHUI/needs" "csrf=$T&action=add&other=$API" >/dev/null
INVITES=$(step Beta+invite+flow task 3 high "$AUTH")
post "/goals/$AUTH/size" "csrf=$T&min=6&max=12" >/dev/null
HOSTING=$(step Hosting+%26+deploy task 4 high "")
post "/nodes/$ENGINE/risks" "csrf=$T&title=Monte+Carlo+too+slow+above+500+nodes&likelihood=40&min=3&max=8" >/dev/null
post "/nodes/$ENGINE/risks" "csrf=$T&title=Correlated+estimates+skew+P85&likelihood=25&min=2&max=5" >/dev/null
post "/goals/$G/shape" "csrf=$T&coverage=partial" >/dev/null
echo "goal $G seeded; cookie jar: $J"
