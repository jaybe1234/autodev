#!/bin/sh
set -e

echo "=== autodev agent starting ==="
echo "JIRA_KEY: $JIRA_KEY"
echo "REPO_URL: $(echo "$REPO_URL" | sed 's|://.*@|://***@|')"
echo "BRANCH:   $BRANCH_NAME"

git clone "$REPO_URL" /workspace/repo
cd /workspace/repo

git checkout -b "$BRANCH_NAME"

git config user.name "autodev[bot]"
git config user.email "autodev[bot]@users.noreply.github.com"

echo "=== running opencode ==="
opencode run "$OPENCODE_PROMPT" \
  --dangerously-skip-permissions

echo "=== autodev agent finished ==="
