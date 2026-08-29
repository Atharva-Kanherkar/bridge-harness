#!/bin/sh
# A stand-in for the Cursor CLI, for the discovery and probe tests in
# bridge-core's cursor_adapter module. It exists so those tests can exercise
# real executables on a real search path, a real handshake, and a real
# pre-protocol build, without depending on the vendor being installed.
#
# Behaviour is chosen entirely by environment variables so one fixture covers
# every case. Request ids are echoed back by extracting the value between
# "id":" and ","method", which is exact: the protocol crate serializes a request
# as {"jsonrpc":"2.0","id":"<uuid>","method":"<name>","params":{...}}.

set -u

version="${BRIDGE_CURSOR_FAKE_VERSION-2026.07.23-e383d2b}"
mode="${BRIDGE_CURSOR_FAKE_MODE:-protocol}"
agent_name="${BRIDGE_CURSOR_FAKE_AGENT_NAME:-Cursor Agent}"

if [ "${1:-}" = "--version" ]; then
    if [ -z "$version" ]; then
        printf 'unknown command\n' >&2
        exit 1
    fi
    printf '%s\n' "$version"
    exit 0
fi

# A build that predates the ACP subcommand does not reject it. It treats "acp"
# as a prompt, starts an interactive terminal session, and paints control
# sequences where a client is waiting for a protocol message. It consumes the
# handshake as if it were typed input.
if [ "$mode" = "terminal" ] || [ "${1:-}" != "acp" ]; then
    printf '\033[?1049h\033[2J\033[H'
    printf 'Cursor Agent %s\n' "$version"
    IFS= read -r _ignored
    printf '\033[1;32m>\033[0m thinking...\n'
    exit 1
fi

initialize_result() {
    printf '{"jsonrpc":"2.0","id":"%s","result":{"protocolVersion":1,' "$1"
    printf '"agentInfo":{"name":"%s","version":"%s"},' "$agent_name" "$version"
    printf '"agentCapabilities":{"loadSession":true,'
    printf '"sessionCapabilities":{"list":{}},'
    printf '"promptCapabilities":{"image":true,"audio":false,"embeddedContext":false}},'
    printf '"authMethods":[{"id":"cursor_login","name":"Log in with Cursor",'
    printf '"description":"Run the login command in a terminal"}]}}\n'
}

# The session's configuration selectors, with the model one set to whichever
# value is passed in. Two selectors, because the vendor surfaces models, modes
# and thought levels through the same mechanism.
config_options() {
    printf '"configOptions":[{"id":"model","name":"Model","category":"model","type":"select",'
    printf '"currentValue":"%s","options":[' "$1"
    printf '{"value":"cheetah","name":"Cheetah"},'
    printf '{"value":"composer-1","name":"Composer 1"},'
    printf '{"value":"claude-4.5-sonnet","name":"Claude 4.5 Sonnet"}]},'
    printf '{"id":"thinking","name":"Thinking","category":"thoughtLevel","type":"boolean",'
    printf '"currentValue":true}]'
}

new_session_result() {
    printf '{"jsonrpc":"2.0","id":"%s","result":{"sessionId":"%s",' \
        "$1" "${BRIDGE_CURSOR_FAKE_SESSION_ID:-cursor-session}"
    printf '"modes":{"currentModeId":"plan","availableModes":['
    printf '{"id":"agent","name":"Agent"},{"id":"plan","name":"Plan"},{"id":"ask","name":"Ask"}]},'
    config_options "composer-1"
    printf '}}\n'
}

answered=0
pending_new=""

while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)","method".*/\1/p')
    case "$line" in
        *'"method":"initialize"'*)
            initialize_result "$id"
            # Cursor issues requests outside the protocol's own vocabulary, and
            # some of them block until answered. This one goes out before the
            # session exists, so the handshake cannot complete unless the client
            # answered it.
            if [ "$mode" = "vendor_traffic" ]; then
                printf '{"jsonrpc":"2.0","id":"cursor-ask-1","method":"cursor/ask_question",'
                printf '"params":{"question":"Which package should I change?"}}\n'
                printf '{"jsonrpc":"2.0","method":"cursor/update_todos",'
                printf '"params":{"todos":[{"id":"1","title":"Read the module"}]}}\n'
            fi
            ;;
        *'"id":"cursor-ask-1"'*)
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
            # Echoed straight back so a test can see the identifier that
            # actually went on the wire, rather than the one it asked for.
            value=$(printf '%s' "$line" | sed -n 's/.*"value":"\([^"]*\)".*/\1/p')
            printf '{"jsonrpc":"2.0","id":"%s","result":{' "$id"
            config_options "$value"
            printf '}}\n'
            printf '{"jsonrpc":"2.0","method":"cursor/config_echo",'
            printf '"params":{"value":"%s"}}\n' "$value"
            ;;
        *'"method":"session/prompt"'*)
            printf '{"jsonrpc":"2.0","id":"%s","result":{"stopReason":"end_turn"}}\n' "$id"
            ;;
        *)
            ;;
    esac
done
