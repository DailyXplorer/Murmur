/**
 * Detects a pull request whose bot review and follow-up fixes are no longer
 * converging. The workflow comments once and applies the `review-loop` label.
 * The job itself stays green: a loop is a signal for a maintainer, not a
 * compile failure.
 */

export const COMPLETED_ROUND_LIMIT = 2;
export const COMMENT_MARKER = "<!-- murmur-review-loop -->";
export const REVIEW_LOOP_LABEL = "review-loop";

const REVIEW_BOTS = new Set(["coderabbitai[bot]", "coderabbitai"]);

export function isReviewBot(login) {
  return REVIEW_BOTS.has(login ?? "");
}

export function isSkipReview(body) {
  const text = body ?? "";
  return (
    text.includes("skip review by coderabbit.ai") ||
    text.includes("does not receive automatic reviews")
  );
}

export function normalizeComment(body) {
  return (body ?? "")
    .replace(/<!--[\s\S]*?-->/g, " ")
    .replace(/https?:\/\/\S+/g, " ")
    .replace(/`[^`]*`/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase()
    .slice(0, 240);
}

function time(value) {
  const parsed = Date.parse(value ?? "");
  return Number.isNaN(parsed) ? undefined : parsed;
}

/**
 * @param {{
 *   reviews?: { user: string, state?: string, body?: string, submittedAt: string }[],
 *   issueComments?: { user: string, body?: string, createdAt: string }[],
 *   reviewComments?: { user: string, body?: string, path?: string, createdAt: string }[],
 *   commits?: { author?: string, isMerge?: boolean, committerDate?: string, authorDate?: string }[],
 * }} input
 */
export function assessReviewLoop(input) {
  /** @type {{ at: number, kind: "review" | "commit" }[]} */
  const events = [];

  for (const review of input.reviews ?? []) {
    if (!isReviewBot(review.user) || review.state === "PENDING") continue;
    if (isSkipReview(review.body)) continue;
    const at = time(review.submittedAt);
    if (at === undefined) continue;
    events.push({ at, kind: "review" });
  }

  for (const comment of input.issueComments ?? []) {
    if (!isReviewBot(comment.user) || isSkipReview(comment.body)) continue;
    if (!(comment.body ?? "").includes("summarize by coderabbit.ai")) continue;
    const at = time(comment.createdAt);
    if (at === undefined) continue;
    events.push({ at, kind: "review" });
  }

  /** @type {number[]} */
  const authorCommits = [];
  for (const commit of input.commits ?? []) {
    if (commit.isMerge || isReviewBot(commit.author)) continue;
    const at = time(commit.committerDate) ?? time(commit.authorDate);
    if (at === undefined) continue;
    authorCommits.push(at);
    events.push({ at, kind: "commit" });
  }

  events.sort(
    (left, right) => left.at - right.at || (left.kind === "review" ? -1 : 1),
  );

  let completedRounds = 0;
  let phase = "idle";
  for (const event of events) {
    if (event.kind === "review") {
      if (phase === "fixed") completedRounds += 1;
      phase = "reviewed";
      continue;
    }
    if (phase === "reviewed") phase = "fixed";
  }

  /** @type {Map<string, number[]>} */
  const groups = new Map();
  for (const comment of input.reviewComments ?? []) {
    if (!isReviewBot(comment.user)) continue;
    const text = normalizeComment(comment.body);
    if (text.length < 24) continue;
    const key = `${comment.path ?? ""}\n${text}`;
    const at = time(comment.createdAt);
    if (at === undefined) continue;
    const times = groups.get(key) ?? [];
    times.push(at);
    groups.set(key, times);
  }

  /** @type {{ path: string, text: string, count: number }[]} */
  const repeated = [];
  for (const [key, times] of groups) {
    if (times.length < 2) continue;
    times.sort((left, right) => left - right);
    const first = times[0];
    const last = times[times.length - 1];
    const commitBetween = authorCommits.some((at) => at > first && at < last);
    if (!commitBetween) continue;
    const newline = key.indexOf("\n");
    repeated.push({
      path: key.slice(0, newline),
      text: key.slice(newline + 1),
      count: times.length,
    });
  }

  return {
    stalled: completedRounds >= COMPLETED_ROUND_LIMIT || repeated.length > 0,
    completedRounds,
    repeated,
  };
}

export function renderLoopComment(result) {
  const repeats =
    result.repeated.length === 0
      ? ""
      : `\nComments that came back after a fix:\n${result.repeated
          .map(
            (item) =>
              `- \`${item.path}\` (${item.count} times): ${item.text.slice(0, 120)}`,
          )
          .join("\n")}\n`;

  return `${COMMENT_MARKER}
## Review loop detected

This pull request has completed ${result.completedRounds} bot review rounds with a fix commit between them.
${repeats}
Stop the mechanical fixes. A maintainer should decide which remaining comments are in scope. Reply on each open thread with the change you will make, or with the reason the thread can be resolved.

Two completed rounds is the limit: bot review, author commit, bot review, author commit, bot review. Further pushes keep the loop going.
`;
}

function mapInput(reviews, issueComments, reviewComments, commits) {
  return {
    reviews: reviews.map((review) => ({
      user: review.user?.login ?? "",
      state: review.state,
      body: review.body ?? "",
      submittedAt: review.submitted_at,
    })),
    issueComments: issueComments.map((comment) => ({
      user: comment.user?.login ?? "",
      body: comment.body ?? "",
      createdAt: comment.created_at,
    })),
    reviewComments: reviewComments.map((comment) => ({
      user: comment.user?.login ?? "",
      body: comment.body ?? "",
      path: comment.path,
      createdAt: comment.created_at,
    })),
    commits: commits.map((commit) => ({
      author: commit.author?.login ?? "",
      isMerge:
        (commit.parents?.length ?? 0) > 1 ||
        (commit.commit?.message ?? "").startsWith("Merge "),
      committerDate: commit.commit?.committer?.date,
      authorDate: commit.commit?.author?.date,
    })),
  };
}

async function ensureLabel(github, owner, repo, pullNumber) {
  const add = () =>
    github.rest.issues.addLabels({
      owner,
      repo,
      issue_number: pullNumber,
      labels: [REVIEW_LOOP_LABEL],
    });
  try {
    await add();
  } catch (error) {
    const status = error?.status;
    if (status !== 404 && status !== 422) throw error;
    await github.rest.issues.createLabel({
      owner,
      repo,
      name: REVIEW_LOOP_LABEL,
      color: "D93F0B",
      description:
        "Review is repeating without converging; a maintainer should decide",
    });
    await add();
  }
}

/**
 * @param {{ github: any, context: any, core: { info: Function, warning: Function } }} tools
 */
export async function flagReviewLoop({ github, context, core }) {
  const pullNumber =
    context.payload?.pull_request?.number ??
    (context.payload?.issue?.pull_request ? context.issue?.number : undefined);
  if (!pullNumber) {
    core.info("Not a pull request");
    return { stalled: false, commented: false };
  }

  const { owner, repo } = context.repo;
  const [reviews, issueComments, reviewComments, commits] = await Promise.all([
    github.paginate(github.rest.pulls.listReviews, {
      owner,
      repo,
      pull_number: pullNumber,
    }),
    github.paginate(github.rest.issues.listComments, {
      owner,
      repo,
      issue_number: pullNumber,
    }),
    github.paginate(github.rest.pulls.listReviewComments, {
      owner,
      repo,
      pull_number: pullNumber,
    }),
    github.paginate(github.rest.pulls.listCommits, {
      owner,
      repo,
      pull_number: pullNumber,
    }),
  ]);

  const result = assessReviewLoop(
    mapInput(reviews, issueComments, reviewComments, commits),
  );
  if (!result.stalled) {
    core.info(
      `No review loop (${result.completedRounds} completed rounds, ${result.repeated.length} repeated comments)`,
    );
    return { stalled: false, commented: false, ...result };
  }

  const body = renderLoopComment(result);
  const existing = issueComments.find((comment) =>
    (comment.body ?? "").includes(COMMENT_MARKER),
  );
  if (existing) {
    await github.rest.issues.updateComment({
      owner,
      repo,
      comment_id: existing.id,
      body,
    });
  } else {
    await github.rest.issues.createComment({
      owner,
      repo,
      issue_number: pullNumber,
      body,
    });
  }

  try {
    await ensureLabel(github, owner, repo, pullNumber);
  } catch (error) {
    core.warning(
      `Could not apply the ${REVIEW_LOOP_LABEL} label: ${error?.message ?? error}`,
    );
  }

  core.info(
    `Flagged review loop (${result.completedRounds} completed rounds, ${result.repeated.length} repeated comments)`,
  );
  return { stalled: true, commented: true, ...result };
}
