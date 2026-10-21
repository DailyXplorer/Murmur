/**
 * Flags a pull request whose bot reviews show a real loop: the same comment
 * coming back after a fix, findings in the same file for three rounds, or the
 * twentieth round. The workflow comments once and applies the `review-loop`
 * label. The job itself stays green: a loop is a signal for a maintainer, not
 * a compile failure.
 */

export const SAME_AREA_ROUNDS = 3;
export const EMERGENCY_ROUNDS = 20;
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
 * Flags a pull request for a maintainer or the lead agent. It never asks
 * authors to stop fixing real bugs, except at the emergency stop.
 *
 * @param {{
 *   reviews?: { id?: number, user: string, state?: string, body?: string, submittedAt: string }[],
 *   reviewComments?: { user: string, body?: string, path?: string, reviewId?: number, inReplyTo?: number }[],
 *   commits?: { author?: string, isMerge?: boolean, committerDate?: string, authorDate?: string }[],
 * }} input
 */
export function assessReviewLoop(input) {
  /** @type {{ at: number, kind: "review" | "commit", id?: number }[]} */
  const events = [];

  // Only submitted reviews count. The walkthrough is one comment edited in
  // place, so its created_at says nothing about when a round happened.
  for (const review of input.reviews ?? []) {
    if (!isReviewBot(review.user) || review.state === "PENDING") continue;
    if (isSkipReview(review.body)) continue;
    const at = time(review.submittedAt);
    if (at === undefined) continue;
    events.push({ at, kind: "review", id: review.id });
  }

  for (const commit of input.commits ?? []) {
    if (commit.isMerge || isReviewBot(commit.author)) continue;
    const at = time(commit.committerDate) ?? time(commit.authorDate);
    if (at === undefined) continue;
    events.push({ at, kind: "commit" });
  }

  events.sort(
    (left, right) => left.at - right.at || (left.kind === "review" ? -1 : 1),
  );

  // A round is the bot reviews between two author pushes.
  let rounds = 0;
  let pushed = true;
  /** @type {Map<number, number>} */
  const roundOfReview = new Map();
  for (const event of events) {
    if (event.kind === "commit") {
      pushed = true;
      continue;
    }
    if (pushed) rounds += 1;
    pushed = false;
    if (event.id !== undefined) roundOfReview.set(event.id, rounds);
  }

  /** @type {Map<string, Set<number>>} */
  const roundsByFinding = new Map();
  /** @type {Map<string, Set<number>>} */
  const roundsByPath = new Map();
  const add = (map, key, round) =>
    map.set(key, (map.get(key) ?? new Set()).add(round));
  for (const comment of input.reviewComments ?? []) {
    // Thread replies answer a finding; they are not a finding.
    if (!isReviewBot(comment.user) || comment.inReplyTo) continue;
    const round = roundOfReview.get(comment.reviewId);
    if (round === undefined) continue;
    const path = comment.path ?? "";
    add(roundsByPath, path, round);
    const text = normalizeComment(comment.body);
    if (text.length >= 24) add(roundsByFinding, `${path}\n${text}`, round);
  }

  const repeated = [...roundsByFinding]
    .filter(([, seen]) => seen.size >= 2)
    .map(([key, seen]) => {
      const newline = key.indexOf("\n");
      return {
        path: key.slice(0, newline),
        text: key.slice(newline + 1),
        count: seen.size,
      };
    });
  const hotspots = [...roundsByPath]
    .filter(([, seen]) => seen.size >= SAME_AREA_ROUNDS)
    .map(([path, seen]) => ({ path, rounds: seen.size }));
  const emergency = rounds >= EMERGENCY_ROUNDS;

  return {
    stalled: emergency || repeated.length > 0 || hotspots.length > 0,
    rounds,
    repeated,
    hotspots,
    emergency,
  };
}

export function renderLoopComment(result) {
  const sections = [];
  if (result.repeated.length > 0) {
    const items = result.repeated.map(
      (item) =>
        `- \`${item.path}\` (${item.count} rounds): ${item.text.slice(0, 120)}`,
    );
    sections.push(`### The same comment came back after a fix

Check whether the fix works or the comment is wrong, then reply on the thread. Do not apply the same change again.

${items.join("\n")}`);
  }
  if (result.hotspots.length > 0) {
    const items = result.hotspots.map(
      (item) => `- \`${item.path}\` (${item.rounds} rounds)`,
    );
    sections.push(`### Findings in the same file for ${SAME_AREA_ROUNDS} or more rounds

Simplify this part, reusing a proven approach, instead of patching it again.

${items.join("\n")}`);
  }
  if (result.emergency) {
    sections.push(`### ${result.rounds} review rounds

Emergency stop: stop pushing and ask a maintainer.`);
  }

  return `${COMMENT_MARKER}
## Review loop flagged

This is a note for a maintainer or the lead agent. Unless the emergency stop applies, keep fixing real bugs, each with a test that fails first, until a review comes back clean. Style suggestions never block.

${sections.join("\n\n")}
`;
}

function mapInput(reviews, reviewComments, commits) {
  return {
    reviews: reviews.map((review) => ({
      id: review.id,
      user: review.user?.login ?? "",
      state: review.state,
      body: review.body ?? "",
      submittedAt: review.submitted_at,
    })),
    reviewComments: reviewComments.map((comment) => ({
      user: comment.user?.login ?? "",
      body: comment.body ?? "",
      path: comment.path,
      reviewId: comment.pull_request_review_id,
      inReplyTo: comment.in_reply_to_id,
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

  const result = assessReviewLoop(mapInput(reviews, reviewComments, commits));
  if (!result.stalled) {
    core.info(
      `No review loop (${result.rounds} rounds, ${result.repeated.length} repeated comments, ${result.hotspots.length} hotspots)`,
    );
    return { stalled: false, commented: false, ...result };
  }

  const body = renderLoopComment(result);
  const existing = issueComments.find((comment) =>
    (comment.body ?? "").includes(COMMENT_MARKER),
  );
  try {
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
  } catch (error) {
    // Pull requests from forks get a read-only token.
    core.warning(
      `Could not comment on the pull request: ${error?.message ?? error}`,
    );
    return { stalled: true, commented: false, ...result };
  }

  try {
    await ensureLabel(github, owner, repo, pullNumber);
  } catch (error) {
    core.warning(
      `Could not apply the ${REVIEW_LOOP_LABEL} label: ${error?.message ?? error}`,
    );
  }

  core.info(
    `Flagged review loop (${result.rounds} rounds, ${result.repeated.length} repeated comments, ${result.hotspots.length} hotspots)`,
  );
  return { stalled: true, commented: true, ...result };
}
