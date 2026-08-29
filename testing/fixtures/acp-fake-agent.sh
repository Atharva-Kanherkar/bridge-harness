#!/bin/sh
# A minimal Agent Client Protocol agent, for the process-lifecycle tests in
# bridge-core's acp_session module. It exists so those tests can watch a real
# child, a real process group, and real pipes without depending on a vendor
# binary being installed.
#
# Behaviour is chosen entirely by environment variables so one fixture covers
# every case. Request ids are echoed back by extracting the value between
# "id":" and ","method", which is exact: the protocol crate serializes a request
# as {"jsonrpc":"2.0","id":"<uuid>","method":"<name>","params":{...}}.

set -u

if [ -n "${BRIDGE_ACP_FAKE_PID_FILE:-}" ]; then
    printf '%s\n' "$$" > "$BRIDGE_ACP_FAKE_PID_FILE"
fi

# A group mate that outlives its parent unless the whole process group is
# signalled. The sleep duration doubles as a unique marker the test can pgrep.
if [ -n "${BRIDGE_ACP_FAKE_GRANDCHILD_SECONDS:-}" ]; then
    /bin/sleep "$BRIDGE_ACP_FAKE_GRANDCHILD_SECONDS" &
fi

idle() {
    while :; do
        /bin/sleep 60
    done
}

case "${BRIDGE_ACP_FAKE_MODE:-serve}" in
    silent)
        idle
        ;;
    banner)
        printf '%s\n' "${BRIDGE_ACP_FAKE_BANNER:-loading shell profile}"
        printf '%s\n' "${BRIDGE_ACP_FAKE_BANNER:-loading shell profile}"
        idle
        ;;
    noisy_exit)
        index=0
        while [ "$index" -lt "${BRIDGE_ACP_FAKE_STDERR_LINES:-4000}" ]; do
            printf 'fake agent stderr %s %s\n' "$index" "${BRIDGE_ACP_FAKE_MARKER:-marker}" >&2
            index=$((index + 1))
        done
        exit "${BRIDGE_ACP_FAKE_EXIT_CODE:-3}"
        ;;
    *)
        ;;
esac

while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)","method".*/\1/p')
    case "$line" in
        *'"method":"initialize"'*)
            printf '{"jsonrpc":"2.0","id":"%s","result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true},"authMethods":[]}}\n' "$id"
            ;;
        *'"method":"session/new"'*)
            printf '{"jsonrpc":"2.0","id":"%s","result":{"sessionId":"%s"}}\n' \
                "$id" "${BRIDGE_ACP_FAKE_SESSION_ID:-fake-session}"
            ;;
        *'"method":"session/prompt"'*)
            if [ "${BRIDGE_ACP_FAKE_DIE_ON_PROMPT:-}" = "1" ]; then
                printf 'the fake agent fell over %s\n' "${BRIDGE_ACP_FAKE_MARKER:-marker}" >&2
                exit "${BRIDGE_ACP_FAKE_EXIT_CODE:-9}"
            fi
            printf '{"jsonrpc":"2.0","id":"%s","result":{"stopReason":"end_turn"}}\n' "$id"
            ;;
        *)
            ;;
    esac
done
