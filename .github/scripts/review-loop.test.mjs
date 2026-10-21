import assert from "node:assert/strict";
import test from "node:test";
import {
  COMMENT_MARKER,
  COMPLETED_ROUND_LIMIT,
  REVIEW_LOOP_LABEL,
  assessReviewLoop,
  flagReviewLoop,
  normalizeComment,
} from "./review-loop.mjs";

const review = (submittedAt, body = "Found a real bug in the paste path.") => ({
  user: "coderabbitai[bot]",
  state: "COMMENTED",
  body,
  submittedAt,
});

const commit = (committerDate, author = "octocat") => ({
  author,
  isMerge: false,
  committerDate,
});

test("one fix and a follow-up review is still in bounds", () => {
  const result = assessReviewLoop({
    reviews: [review("2026-10-21T00:00:00Z"), review("2026-10-21T00:20:00Z")],
    commits: [commit("2026-10-21T00:10:00Z")],
  });
  assert.equal(result.completedRounds, 1);
  assert.equal(result.stalled, false);
});

test("a third bot review after two fixes is a loop", () => {
  const result = assessReviewLoop({
    reviews: [
      review("2026-10-21T00:00:00Z"),
      review("2026-10-21T00:20:00Z"),
      review("2026-10-21T00:40:00Z"),
    ],
    commits: [commit("2026-10-21T00:10:00Z"), commit("2026-10-21T00:30:00Z")],
  });
  assert.equal(result.completedRounds, COMPLETED_ROUND_LIMIT);
  assert.equal(result.stalled, true);
});

test("a summary comment posted with the review counts as one round", () => {
  const result = assessReviewLoop({
    reviews: [review("2026-10-21T00:00:00Z")],
    issueComments: [
      {
        user: "coderabbitai[bot]",
        body: "<!-- This is an auto-generated comment: summarize by coderabbit.ai -->\nWalkthrough",
        createdAt: "2026-10-21T00:00:30Z",
      },
    ],
    commits: [commit("2026-10-21T00:10:00Z")],
  });
  assert.equal(result.completedRounds, 0);
  assert.equal(result.stalled, false);
});

test("the free-tier skip comment does not start a round", () => {
  const result = assessReviewLoop({
    reviews: [
      review(
        "2026-10-21T00:00:00Z",
        "This repository does not receive automatic reviews because it has fewer than 10 stars.",
      ),
    ],
    issueComments: [
      {
        user: "coderabbitai[bot]",
        body: "<!-- skip review by coderabbit.ai --> fewer than 10 stars",
        createdAt: "2026-10-21T00:00:00Z",
      },
    ],
    commits: [commit("2026-10-21T00:10:00Z"), commit("2026-10-21T00:20:00Z")],
  });
  assert.equal(result.completedRounds, 0);
  assert.equal(result.stalled, false);
});

test("merge commits and CodeRabbit commits do not count as fixes", () => {
  const result = assessReviewLoop({
    reviews: [review("2026-10-21T00:00:00Z"), review("2026-10-21T00:30:00Z")],
    commits: [
      {
        author: "octocat",
        isMerge: true,
        committerDate: "2026-10-21T00:10:00Z",
      },
      commit("2026-10-21T00:20:00Z", "coderabbitai[bot]"),
    ],
  });
  assert.equal(result.completedRounds, 0);
});

test("the same inline comment after a fix is a loop even before the round limit", () => {
  const body =
    "This reads past the end of the sample buffer when the recording is shorter than one frame.";
  const result = assessReviewLoop({
    reviews: [review("2026-10-21T00:00:00Z"), review("2026-10-21T00:20:00Z")],
    commits: [commit("2026-10-21T00:10:00Z")],
    reviewComments: [
      {
        user: "coderabbitai[bot]",
        path: "src-tauri/src/audio_toolkit/gain.rs",
        body,
        createdAt: "2026-10-21T00:00:00Z",
      },
      {
        user: "coderabbitai[bot]",
        path: "src-tauri/src/audio_toolkit/gain.rs",
        body: `${body}\n\n<!-- line 40 -->`,
        createdAt: "2026-10-21T00:20:00Z",
      },
    ],
  });
  assert.equal(result.completedRounds, 1);
  assert.equal(result.repeated.length, 1);
  assert.equal(result.stalled, true);
});

test("identical comments inside one review are not a repeat", () => {
  const body =
    "This reads past the end of the sample buffer when the recording is shorter than one frame.";
  const result = assessReviewLoop({
    reviews: [review("2026-10-21T00:00:00Z")],
    reviewComments: [
      {
        user: "coderabbitai[bot]",
        path: "src-tauri/src/lib.rs",
        body,
        createdAt: "2026-10-21T00:00:00Z",
      },
      {
        user: "coderabbitai[bot]",
        path: "src-tauri/src/lib.rs",
        body,
        createdAt: "2026-10-21T00:00:05Z",
      },
    ],
  });
  assert.equal(result.repeated.length, 0);
  assert.equal(result.stalled, false);
});

test("normalizeComment ignores links, code, and html comments", () => {
  assert.equal(
    normalizeComment("See `foo()` at https://example.com <!-- note --> now"),
    "see at now",
  );
});

function githubFixture(state) {
  const notFound = () => {
    const error = new Error("Label does not exist");
    error.status = 404;
    throw error;
  };
  return {
    paginate: async (fn, args) => fn(args),
    rest: {
      pulls: {
        listReviews: async () => state.reviews,
        listReviewComments: async () => state.reviewComments,
        listCommits: async () => state.commits,
      },
      issues: {
        listComments: async () => state.issueComments,
        createComment: async ({ body }) => {
          const comment = {
            id: state.issueComments.length + 1,
            body,
            user: { login: "github-actions[bot]" },
          };
          state.issueComments.push(comment);
          return { data: comment };
        },
        updateComment: async ({ comment_id, body }) => {
          const comment = state.issueComments.find(
            (item) => item.id === comment_id,
          );
          comment.body = body;
          state.updates += 1;
          return { data: comment };
        },
        addLabels: async ({ labels }) => {
          if (!state.labelExists) notFound();
          state.labels.push(...labels);
        },
        createLabel: async ({ name }) => {
          state.createdLabels.push(name);
          state.labelExists = true;
        },
      },
    },
  };
}

function stalledState() {
  return {
    reviews: [
      {
        user: { login: "coderabbitai[bot]" },
        state: "COMMENTED",
        body: "bug",
        submitted_at: "2026-10-21T00:00:00Z",
      },
      {
        user: { login: "coderabbitai[bot]" },
        state: "COMMENTED",
        body: "bug",
        submitted_at: "2026-10-21T00:20:00Z",
      },
      {
        user: { login: "coderabbitai[bot]" },
        state: "COMMENTED",
        body: "bug",
        submitted_at: "2026-10-21T00:40:00Z",
      },
    ],
    reviewComments: [],
    issueComments: [],
    commits: [
      {
        author: { login: "octocat" },
        parents: [{}],
        commit: {
          message: "fix: first",
          committer: { date: "2026-10-21T00:10:00Z" },
        },
      },
      {
        author: { login: "octocat" },
        parents: [{}],
        commit: {
          message: "fix: second",
          committer: { date: "2026-10-21T00:30:00Z" },
        },
      },
    ],
    labels: [],
    createdLabels: [],
    labelExists: false,
    updates: 0,
  };
}

const context = {
  payload: { pull_request: { number: 7 } },
  issue: { number: 7 },
  repo: { owner: "DailyXplorer", repo: "Murmur" },
};

test("flagReviewLoop comments once, creates the label, and updates the comment", async () => {
  const state = stalledState();
  const core = { info() {}, warning() {} };
  const first = await flagReviewLoop({
    github: githubFixture(state),
    context,
    core,
  });
  assert.equal(first.commented, true);
  assert.equal(state.issueComments.length, 1);
  assert.match(state.issueComments[0].body, new RegExp(COMMENT_MARKER));
  assert.deepEqual(state.createdLabels, [REVIEW_LOOP_LABEL]);
  assert.deepEqual(state.labels, [REVIEW_LOOP_LABEL]);

  const second = await flagReviewLoop({
    github: githubFixture(state),
    context,
    core,
  });
  assert.equal(second.commented, true);
  assert.equal(state.issueComments.length, 1);
  assert.equal(state.updates, 1);
});

test("flagReviewLoop stays quiet when the review is converging", async () => {
  const state = stalledState();
  state.reviews = state.reviews.slice(0, 1);
  state.commits = [];
  const result = await flagReviewLoop({
    github: githubFixture(state),
    context,
    core: { info() {}, warning() {} },
  });
  assert.equal(result.stalled, false);
  assert.equal(state.issueComments.length, 0);
  assert.equal(state.labels.length, 0);
});

test("flagReviewLoop ignores issues that are not pull requests", async () => {
  const result = await flagReviewLoop({
    github: githubFixture(stalledState()),
    context: {
      payload: { issue: {} },
      issue: { number: 3 },
      repo: context.repo,
    },
    core: { info() {}, warning() {} },
  });
  assert.equal(result.commented, false);
});
