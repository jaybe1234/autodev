#!/bin/sh
set -e

echo "=== autodev agent starting ==="
echo "MODE:     ${MODE:-impl}"
echo "JIRA_KEY: $JIRA_KEY"
echo "REPO_URL: $(echo "$REPO_URL" | sed 's|://.*@|://***@|')"
echo "BRANCH:   $BRANCH_NAME"

git clone "$REPO_URL" /workspace/repo
cd /workspace/repo

git config user.name "autodev[bot]"
git config user.email "autodev[bot]@users.noreply.github.com"

if [ "$MODE" = "review" ]; then
    echo "=== review mode: checking out existing branch ==="
    git fetch origin "$BRANCH_NAME"
    git checkout "$BRANCH_NAME"

    echo "=== running opencode (resuming session) ==="
    opencode run --session "$OPENCODE_SESSION_ID" "$OPENCODE_PROMPT" \
        --dangerously-skip-permissions
else
    echo "=== implementation mode: creating new branch ==="
    git checkout -b "$BRANCH_NAME"

    echo "=== running opencode ==="
    opencode run "$OPENCODE_PROMPT" \
        --dangerously-skip-permissions
fi

echo "=== capturing session ID ==="
SESSION_ID=$(opencode session list --format json 2>/dev/null \
    | jq -r 'sort_by(.createdAt) | .[0].id // empty' 2>/dev/null || true)
if [ -n "$SESSION_ID" ]; then
    echo "OPENCODE_SESSION_ID=${SESSION_ID}"
fi

echo "=== autodev agent finished ==="
