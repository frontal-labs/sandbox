#!/bin/bash
# usage: run.sh <bin> <n> <payload> <mode> <port> <label>
BIN=$1; N=$2; PAYLOAD=$3; MODE=$4; PORT=$5; LABEL=$6
S=$(dirname "$0")
D=$S/data-idle-$LABEL; rm -rf "$D"; mkdir -p "$D"
KEYS=$(( (N + 29) / 30 + 1 ))
RUST_LOG=warn "$BIN" serve "$D/state" 127.0.0.1:$PORT "$D/keys" $KEYS > "$D/server.log" 2>&1 &
PID=$!
for i in $(seq 1 100); do [ -s "$D/keys" ] && nc -z 127.0.0.1 $PORT && break; /bin/sleep 0.2; done
echo "label=$LABEL server_pid=$PID keys=$KEYS n=$N payload=$PAYLOAD mode=$MODE"
"$BIN" drive 127.0.0.1:$PORT "$D/keys" $PID $N $PAYLOAD $MODE "$D/state" 2>&1 | tee "$D/drive.log"
echo "server_log_lines=$(wc -l < $D/server.log) warn_idle_sleep_failed=$(grep -c 'idle sleep failed' $D/server.log)"
kill $PID; wait $PID 2>/dev/null
