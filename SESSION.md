# SESSION.md

Deferred issues, open questions, and known limitations encountered during
development. These do not block the initial implementation but should be
addressed in future iterations.

## Open questions

- **Rate limiting / concurrency** — No concurrency limit on containers yet. A
  semaphore or job queue should be added to bound the number of simultaneous
  agent containers.

- **Container timeout** — No mechanism to kill containers that run too long. A
  configurable timeout (e.g., 30 minutes) should be enforced, after which the
  container is killed and the task is marked failed.

- **Branch cleanup** — Old `autodev/` branches accumulate in repos over time.
  Consider a cleanup policy (e.g., delete branch after PR is merged).

- **Multiple labels** — If a Jira ticket has labels matching more than one
  `[[mapping]]` entry, only the first match is used today. A future iteration
  could spawn one container per matching label, or require an unambiguous
  mapping.

- **Inline review comments** — Only the review body (summary comment) is
  forwarded to the agent. Inline `pull_request_review_comment` events with
  line-specific feedback are not handled yet.

- **Review queue persistence** — The review queue is in-memory
  (`Mutex<HashMap>`). If the server restarts, queued reviews are lost. Consider
  persisting them in SQLite or accepting this as acceptable since GitHub retries
  webhook deliveries.

- **Branch name for comment-triggered reviews** — When an `issue_comment`
  triggers a review, the branch name is derived from the original task's
  `jira_key` (`autodev/<key>`). This could break if the PR branch was renamed
  manually. A more robust approach would store the branch name in the DB when
  the original task completes.
