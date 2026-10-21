import assert from "node:assert/strict";
import test from "node:test";
import {
  COMMENT_MARKER,
  REVIEW_LOOP_LABEL,
  assessReviewLoop,
  flagReviewLoop,
  normalizeComment,
} from "./review-loop.mjs";

const BOT = "coderabbitai[bot]";
const BUG =
  "This reads past the end of the sample buffer when the recording is shorter than one frame.";

const at = (minute) => `2026-10-21T00:${String(minute).padStart(2, "0")}:00Z`;

// Each review covers its own pushed head unless a test says otherwise.
const review = (
  id,
  submittedAt,
  commitId = `sha${id}`,
  body = "**Actionable comments posted: 1**",
) => ({
  id,
  user: BOT,
  state: "COMMENTED",
  body,
  submittedAt,
  commitId,
});

// createdAt mirrors the API; the detector ignores it and groups by review id.
const finding = (reviewId, path, body, inReplyTo) => ({
  user: BOT,
  reviewId,
  path,
  body,
  inReplyTo,
  createdAt: at((reviewId - 1) * 20),
});

test("one fix and a follow-up review is two rounds and no loop", () => {
  const result = assessReviewLoop({
    reviews: [review(1, at(0)), review(2, at(20))],
  });
  assert.equal(result.rounds, 2);
  assert.equal(result.stalled, false);
});

test("new findings in different files every round never flag", () => {
  const result = assessReviewLoop({
    reviews: [review(1, at(0)), review(2, at(20)), review(3, at(40))],
    reviewComments: [
      finding(1, "src-tauri/src/a.rs", `${BUG} a`),
      finding(2, "src-tauri/src/b.rs", `${BUG} b`),
      finding(3, "src-tauri/src/c.rs", `${BUG} c`),
    ],
  });
  assert.equal(result.rounds, 3);
  assert.equal(result.stalled, false);
});

test("a clean review after two fix rounds is convergence, not a loop", () => {
  const result = assessReviewLoop({
    reviews: [review(1, at(0)), review(2, at(20)), review(3, at(40))],
    reviewComments: [
      finding(1, "src/a.ts", `${BUG} a`),
      finding(2, "src/b.ts", `${BUG} b`),
    ],
  });
  assert.equal(result.stalled, false);
});

test("findings in the same file in three rounds flag the area for simplification", () => {
  const path = "src-tauri/src/audio_toolkit/gain.rs";
  const result = assessReviewLoop({
    reviews: [review(1, at(0)), review(2, at(20)), review(3, at(40))],
    reviewComments: [1, 2, 3].map((id) =>
      finding(id, path, `${BUG} variant ${id}`),
    ),
  });
  assert.deepEqual(result.hotspots, [{ path, rounds: 3 }]);
  assert.equal(result.repeated.length, 0);
  assert.equal(result.stalled, true);
});

test("a bot reply in a thread is not a comment coming back", () => {
  const reply =
    "Thanks for the fix, the bounds check now covers the short recording case.";
  const result = assessReviewLoop({
    reviews: [review(1, at(0)), review(2, at(20))],
    reviewComments: [
      finding(1, "src/a.ts", reply, 100),
      finding(2, "src/a.ts", reply, 101),
    ],
  });
  assert.equal(result.repeated.length, 0);
  assert.equal(result.stalled, false);
});

test("twenty rounds is the emergency stop", () => {
  const reviews = [];
  for (let i = 0; i < 20; i += 1) {
    const hour = String(i).padStart(2, "0");
    reviews.push(review(i + 1, `2026-10-21T${hour}:00:00Z`));
  }
  const result = assessReviewLoop({ reviews });
  assert.equal(result.emergency, true);
  assert.equal(result.stalled, true);
});

test("the free-tier skip review does not start a round", () => {
  const result = assessReviewLoop({
    reviews: [
      review(
        1,
        at(0),
        "sha1",
        "This repository does not receive automatic reviews because it has fewer than 10 stars.",
      ),
    ],
  });
  assert.equal(result.rounds, 0);
  assert.equal(result.stalled, false);
});

test("reviews of the same pushed head are one round", () => {
  const result = assessReviewLoop({
    reviews: [review(1, at(0), "sha1"), review(2, at(30), "sha1")],
  });
  assert.equal(result.rounds, 1);
});

test("the same comment twice on one head is not a repeat", () => {
  const result = assessReviewLoop({
    reviews: [review(1, at(0), "sha1"), review(2, at(20), "sha1")],
    reviewComments: [finding(1, "src/a.ts", BUG), finding(2, "src/a.ts", BUG)],
  });
  assert.equal(result.repeated.length, 0);
  assert.equal(result.stalled, false);
});

test("the same inline comment after a fix is a loop", () => {
  const path = "src-tauri/src/audio_toolkit/gain.rs";
  const result = assessReviewLoop({
    reviews: [review(1, at(0)), review(2, at(20))],
    reviewComments: [
      finding(1, path, BUG),
      finding(2, path, `${BUG}\n\n<!-- line 40 -->`),
    ],
  });
  assert.equal(result.rounds, 2);
  assert.equal(result.repeated.length, 1);
  assert.equal(result.stalled, true);
});

test("identical comments inside one review are not a repeat", () => {
  const result = assessReviewLoop({
    reviews: [review(1, at(0))],
    reviewComments: [
      finding(1, "src-tauri/src/lib.rs", BUG),
      finding(1, "src-tauri/src/lib.rs", BUG),
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
    reviews: [1, 2, 3].map((id) => ({
      id,
      user: { login: "coderabbitai[bot]" },
      state: "COMMENTED",
      body: "bug",
      submitted_at: at((id - 1) * 20),
      commit_id: `sha${id}`,
    })),
    reviewComments: [1, 3].map((reviewId) => ({
      user: { login: "coderabbitai[bot]" },
      path: "src-tauri/src/audio_toolkit/gain.rs",
      body: BUG,
      pull_request_review_id: reviewId,
    })),
    issueComments: [],
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

test("a fork pull request with a read-only token keeps the job green", async () => {
  const state = stalledState();
  const github = githubFixture(state);
  const forbidden = async () => {
    const error = new Error("Resource not accessible by integration");
    error.status = 403;
    throw error;
  };
  github.rest.issues.createComment = forbidden;
  github.rest.issues.addLabels = forbidden;
  const result = await flagReviewLoop({
    github,
    context,
    core: { info() {}, warning() {} },
  });
  assert.equal(result.stalled, true);
  assert.equal(result.commented, false);
});

test("flagReviewLoop does not overwrite a pasted marker from someone else", async () => {
  const state = stalledState();
  state.issueComments.push({
    id: 1,
    body: `quoting ${COMMENT_MARKER}`,
    user: { login: "octocat" },
  });
  await flagReviewLoop({
    github: githubFixture(state),
    context,
    core: { info() {}, warning() {} },
  });
  assert.equal(state.updates, 0);
  assert.equal(state.issueComments[0].body, `quoting ${COMMENT_MARKER}`);
  assert.equal(state.issueComments.length, 2);
});
