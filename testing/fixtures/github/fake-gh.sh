#!/bin/sh
# Immutable executable shared by tests. Each symlink's directory owns its data.
root=$(dirname "$0")
fixtures=$(cat "$root/fixture-root")
printf '%s\n' "$*" >> "$root/invocations.log"

if [ "$1 $2" = "auth status" ]; then
    exit "$(cat "$root/auth-exit")"
fi
if [ -f "$root/hang" ]; then
    sleep 30
    exit 0
fi
if [ -f "$root/allow-all" ]; then
    exit 0
fi
if [ -f "$root/no-merge-strategies" ]; then
    printf '%s\n' '{"allow_merge_commit":false,"allow_squash_merge":false,"allow_rebase_merge":false}'
    exit 0
fi
if [ "$1 $2 $3" = "repo set-default --view" ]; then
    if [ -s "$root/default-repository" ]; then
        cat "$root/default-repository"
        exit 0
    fi
    exit 1
fi
if [ "$1 $2" = "pr list" ]; then
    case "$*" in
        *statusCheckRollup*)
            if [ -f "$root/pr-list-rich-slow" ]; then sleep 6; fi
            if [ -f "$root/pr-list-rich-fail" ]; then
                echo 'HTTP 502: 502 Bad Gateway (https://api.github.com/graphql)' >&2
                exit 1
            fi
            fixture=prs.json
            ;;
        *) fixture=prs-base.json ;;
    esac
    if [ -f "$root/partial-rerun-failure" ]; then fixture=prs.json; fi
    if [ -f "$root/pr-list-fixture" ]; then fixture=$(cat "$root/pr-list-fixture"); fi
    cat "$fixtures/$fixture"
    exit 0
fi
if [ "$1 $2" = "pr view" ]; then cat "$fixtures/pr-detail.json"; exit 0; fi
if [ "$1 $2" = "issue list" ]; then cat "$fixtures/issues.json"; exit 0; fi
if [ "$1 $2" = "issue view" ]; then cat "$fixtures/issue-detail.json"; exit 0; fi
if [ "$1 $2" = "repo view" ]; then cat "$fixtures/repository.json"; exit 0; fi
if [ "$1 $2" = "label list" ]; then cat "$fixtures/labels.json"; exit 0; fi
if [ "$1 $2" = "pr checks" ]; then
    if [ -f "$root/pr-checks-empty" ]; then
        echo "no checks reported on the 'fixture' branch" >&2
        exit 1
    fi
    cat "$fixtures/checks.json"
    exit 1
fi
if [ "$1 $2" = "api graphql" ]; then cat "$fixtures/review-threads.json"; exit 0; fi
if [ "$1 $2" = "pr merge" ]; then
    if [ -f "$root/pr-merge-blocked" ]; then
        echo 'GraphQL: Branch protections: at least 1 approving review is required (mergePullRequest)' >&2
        exit 1
    fi
    exit 0
fi
if [ "$1 $2" = "pr review" ] || [ "$1 $2" = "pr comment" ] || [ "$1 $2" = "pr edit" ] || [ "$1 $2" = "issue edit" ]; then exit 0; fi
if [ "$1 $2" = "issue comment" ] || [ "$1 $2" = "pr ready" ]; then exit 0; fi
if [ "$2" = "close" ] || [ "$2" = "reopen" ]; then exit 0; fi
if [ "$1 $2" = "run list" ]; then
    if [ -f "$root/partial-rerun-failure" ]; then
        printf '%s\n' '[{"databaseId":42,"conclusion":"failure"},{"databaseId":43,"conclusion":"failure"}]'
        exit 0
    fi
    fixture=runs.json
    if [ -f "$root/run-list-clean" ]; then fixture=runs-clean.json; fi
    cat "$fixtures/$fixture"
    exit 0
fi
if [ "$1 $2" = "run rerun" ]; then
    if [ -f "$root/partial-rerun-failure" ] && [ "$3" = "43" ]; then
        echo 'second rerun refused' >&2
        exit 1
    fi
    exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "--method" ]; then exit 0; fi
if [ "$1" = "api" ]; then
    case "$4" in repos/*/pulls/*/files*) cat "$fixtures/pr-files.json"; exit 0 ;; esac
    case "$2" in repos/*) cat "$fixtures/repos-settings.json"; exit 0 ;; esac
fi
exit 2
