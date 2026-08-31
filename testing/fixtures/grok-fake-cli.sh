#!/bin/sh
# A stand-in for the xAI Grok Build CLI (`grok`), for the discovery, probe,
# and session execution tests in bridge-core's grok_adapter module.
#
# Behaviour is controlled by environment variables and flags so one fixture
# covers every test case.

set -u

version="${BRIDGE_GROK_FAKE_VERSION-1.0.4-e2b819f}"
mode="${BRIDGE_GROK_FAKE_MODE:-protocol}"
agent_name="${BRIDGE_GROK_FAKE_AGENT_NAME:-Grok Build}"

if [ "${1:-}" = "--version" ]; then
    if [ -z "$version" ]; then
        printf 'unknown command\n' >&2
        exit 1
    fi
    printf '%s\n' "$version"
    exit 0
fi

# Verify the launch arguments. Grok ACP launch is expected to be:
# grok agent --no-leader stdio
if [ "$mode" = "terminal" ] || [ "${1:-}" != "agent" ]; then
    printf '\033[?1049h\033[2J\033[H'
    printf 'Grok Build %s\n' "$version"
    IFS= read -r _ignored
    printf '\033[1;32m>\033[0m thinking...\n'
    exit 1
fi

# If mode requires validating --no-leader
if [ "${2:-}" != "--no-leader" ] || [ "${3:-}" != "stdio" ]; then
    if [ "$mode" = "reject_missing_no_leader" ] || [ "${BRIDGE_GROK_REQUIRE_NO_LEADER:-1}" = "1" ]; then
        printf 'Error: bridge requires "agent --no-leader stdio" for supervised execution\n' >&2
        exit 1
    fi
fi

initialize_result() {
    printf '{"jsonrpc":"2.0","id":"%s","result":{"protocolVersion":1,' "$1"
    printf '"agentInfo":{"name":"%s","version":"%s"},' "$agent_name" "$version"
    printf '"agentCapabilities":{"loadSession":true,'
    printf '"sessionCapabilities":{"list":{}},'
    printf '"promptCapabilities":{"image":true,"audio":false,"embeddedContext":false}},'
    printf '"authMethods":[{"id":"grok_login","name":"Log in with xAI Grok",'
    printf '"description":"Run grok login in a terminal"}]}}\n'
}

config_options() {
    printf '"configOptions":[{"id":"model","name":"Model","category":"model","type":"select",'
    printf '"currentValue":"%s","options":[' "$1"
    printf '{"value":"grok-code","name":"Grok Code"},'
    printf '{"value":"grok-3","name":"Grok 3"},'
    printf '{"value":"grok-3-mini","name":"Grok 3 Mini"}]},'
    printf '{"id":"thinking","name":"Thinking","category":"thoughtLevel","type":"boolean",'
    printf '"currentValue":true}]'
}

new_session_result() {
    printf '{"jsonrpc":"2.0","id":"%s","result":{"sessionId":"%s",' \
        "$1" "${BRIDGE_GROK_FAKE_SESSION_ID:-grok-session-1}"
    printf '"modes":{"currentModeId":"agent","availableModes":['
    printf '{"id":"agent","name":"Agent"},{"id":"plan","name":"Plan"},{"id":"ask","name":"Ask"}]},'
    config_options "${BRIDGE_GROK_FAKE_DEFAULT_MODEL:-grok-code}"
    printf '}}\n'
}

answered=0
pending_new=""

while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)","method".*/\1/p')
    case "$line" in
        *'"method":"initialize"'*)
            initialize_result "$id"
            if [ "$mode" = "vendor_traffic" ]; then
                printf '{"jsonrpc":"2.0","id":"grok-ask-1","method":"grok/status_check",'
                printf '"params":{"status":"ready"}}\n'
            fi
            ;;
        *'"id":"grok-ask-1"'*)
            answered=1
            if [ -n "$pending_new" ]; then
                new_session_result "$pending_new"
                pending_new=""
            fi
            ;;
        *'"method":"session/new"'*)
            case "$mode" in
                needs_login)
                    printf '{"jsonrpc":"2.0","id":"%s","error":{"code":-32000,' "$id"
                    printf '"message":"Authentication required"}}\n'
                    ;;
                vendor_traffic)
                    if [ "$answered" = "1" ]; then
                        new_session_result "$id"
                    else
                        pending_new="$id"
                    fi
                    ;;
                *)
                    new_session_result "$id"
                    ;;
            esac
            ;;
        *'"method":"session/set_config_option"'*)
            value=$(printf '%s' "$line" | sed -n 's/.*"value":"\([^"]*\)".*/\1/p')
            printf '{"jsonrpc":"2.0","id":"%s","result":{' "$id"
            config_options "$value"
            printf '}}\n'
            ;;
        *'"method":"session/prompt"'*)
            # Send sample turn stream and finish
            printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"grok-session-1","update":{"type":"text","content":"Hello from Grok"}}}\n'
            printf '{"jsonrpc":"2.0","id":"%s","result":{"stopReason":"end_turn"}}\n' "$id"
            ;;
        *'"method":"session/cancel"'*)
            printf '{"jsonrpc":"2.0","id":"%s","result":{}}\n' "$id"
            ;;
    esac
done
