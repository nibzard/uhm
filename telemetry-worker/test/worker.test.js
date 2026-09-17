import assert from "node:assert/strict";
import test from "node:test";
import { eventToPoint, handle, validateEvent } from "../src/index.js";

function event(overrides = {}) {
  return {
    v: 2,
    event: "interaction_summary",
    release: "0.1",
    os: "linux",
    arch: "x86_64",
    shell: "bash",
    mode: "auto",
    route: "shell",
    decision: "ran",
    effects: "read_local",
    proposal_outcome: "valid",
    execution_outcome: "exit_zero",
    user_feedback: "unknown",
    latency: "1s_2s",
    cache: "miss",
    parent_action: "not_applicable",
    interactive: true,
    notice_revision: 5,
    ...overrides,
  };
}

function environment({ enabled = "true", rate = true } = {}) {
  const points = [];
  return {
    points,
    env: {
      ENABLED: enabled,
      RATE_LIMITER: { limit: async () => ({ success: rate }) },
      EVENTS: { writeDataPoint: (point) => points.push(point) },
    },
  };
}

function request(value, headers = {}) {
  return new Request("https://telemetry.test/v1/events", {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: typeof value === "string" ? value : JSON.stringify(value),
  });
}

test("accepts the exact v2 schema and preserves WAE ordering", async () => {
  const { env, points } = environment();
  const response = await handle(request(event()), env);
  assert.equal(response.status, 202);
  assert.equal(points.length, 1);
  assert.deepEqual(points[0], eventToPoint(event()));
  assert.deepEqual(points[0].indexes, ["interaction_summary"]);
  assert.equal(points[0].blobs[5], "shell");
  assert.deepEqual(points[0].doubles, [1, 5, 2]);
  assert.equal(validateEvent(event({ route: "program", execution_outcome: "output_overflow" })), true);
});

test("continues accepting the released exact v1 schema", () => {
  const legacy = event({ v: 1, notice_revision: 2 });
  delete legacy.parent_action;
  assert.equal(validateEvent(legacy), true);
  assert.equal(eventToPoint(legacy).blobs[13], "not_applicable");
});

test("rejects unknown keys, values, versions, and arbitrary strings", async () => {
  for (const invalid of [
    event({ prompt: "private" }),
    event({ route: "/home/person/repository" }),
    event({ v: 3 }),
    event({ notice_revision: 6 }),
    event({ interactive: "yes" }),
  ]) {
    assert.equal(validateEvent(invalid), false);
    const { env, points } = environment();
    assert.equal((await handle(request(invalid), env)).status, 422);
    assert.equal(points.length, 0);
  }
});

test("enforces method, path, content type, and body size", async () => {
  const { env } = environment();
  assert.equal((await handle(new Request("https://telemetry.test/v1/events"), env)).status, 404);
  assert.equal((await handle(new Request("https://telemetry.test/nope", { method: "POST" }), env)).status, 404);
  assert.equal((await handle(request(event(), { "content-type": "text/plain" }), env)).status, 415);
  assert.equal((await handle(request("x".repeat(2048)), env)).status, 413);
});

test("kill switch and rate limiter reject without writing", async () => {
  const killed = environment({ enabled: "false" });
  assert.equal((await handle(request(event()), killed.env)).status, 503);
  assert.equal(killed.points.length, 0);
  const limited = environment({ rate: false });
  assert.equal((await handle(request(event()), limited.env)).status, 429);
  assert.equal(limited.points.length, 0);
});

test("accepts the expanded v2 schema the shipped client sends", async () => {
  for (const expansion_outcome of ["none", "probed", "probe_empty", "invalid_probe"]) {
    const expanded = event({ expansion_outcome });
    assert.equal(validateEvent(expanded), true);
    const { env, points } = environment();
    const response = await handle(request(expanded), env);
    assert.equal(response.status, 202);
    assert.equal(points.length, 1);
    assert.deepEqual(points[0], eventToPoint(expanded));
    // Existing column positions are unchanged; expansion is appended as blob15.
    assert.equal(points[0].blobs[0], expanded.release);
    assert.equal(points[0].blobs[13], expanded.parent_action);
    assert.equal(points[0].blobs[14], expansion_outcome);
    assert.deepEqual(points[0].doubles, [1, 5, 2]);
  }
  // A payload without the expanded field stays the legacy v2 shape, and its
  // projected point normalizes the absent expansion to none.
  const legacy = event();
  assert.equal(validateEvent(legacy), true);
  assert.equal(eventToPoint(legacy).blobs[14], "none");
  const v1 = event({ v: 1, notice_revision: 2 });
  delete v1.parent_action;
  assert.equal(eventToPoint(v1).blobs[14], "none");
});

test("rejects invalid expansion values, types, and misplaced fields", async () => {
  const v1 = event({ v: 1, notice_revision: 2 });
  delete v1.parent_action;
  for (const invalid of [
    event({ expansion_outcome: "sometimes" }),
    event({ expansion_outcome: 5 }),
    event({ expansion_outcome: null }),
    event({ expansion_outcome: ["none"] }),
    { ...v1, expansion_outcome: "none" },
  ]) {
    assert.equal(validateEvent(invalid), false, JSON.stringify(invalid));
    const { env, points } = environment();
    assert.equal((await handle(request(invalid), env)).status, 422);
    assert.equal(points.length, 0);
  }
});

const encoder = new TextEncoder();

function streamRequest(chunks, headers = {}, callbacks = {}) {
  const body = new ReadableStream({
    start(controller) {
      for (const chunk of chunks) {
        controller.enqueue(typeof chunk === "string" ? encoder.encode(chunk) : chunk);
      }
      controller.close();
    },
    cancel() {
      if (callbacks.onCancel) callbacks.onCancel();
    },
  });
  return new Request("https://telemetry.test/v1/events", {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body,
    duplex: "half",
  });
}

test("reads streamed bodies up to one byte below the limit", async () => {
  const payload = JSON.stringify(event({ expansion_outcome: "none" }));
  const padded = payload + " ".repeat(2047 - payload.length);
  assert.equal(padded.length, 2047);
  const { env, points } = environment();
  const response = await handle(streamRequest([padded]), env);
  assert.equal(response.status, 202);
  assert.equal(points.length, 1);
  assert.equal(points[0].blobs[14], "none");
});

test("stops an oversized stream at the limit instead of draining it", async () => {
  // A pull-based source that never closes: correct behavior cancels after
  // the second chunk crosses the limit; draining would hang the test.
  let pulls = 0;
  let cancelled = false;
  const body = new ReadableStream({
    pull(controller) {
      pulls += 1;
      if (pulls === 1) controller.enqueue(encoder.encode("a".repeat(1500)));
      else if (pulls === 2) controller.enqueue(encoder.encode("b".repeat(1500)));
    },
    cancel() {
      cancelled = true;
    },
  });
  const { env, points } = environment();
  const response = await handle(
    new Request("https://telemetry.test/v1/events", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
      duplex: "half",
    }),
    env,
  );
  assert.equal(response.status, 413);
  assert.equal(points.length, 0);
  assert.equal(cancelled, true, "the reader must be cancelled, not drained");
});

test("rejects a single already-delivered oversized chunk immediately", async () => {
  let cancelled = false;
  const body = new ReadableStream({
    pull(controller) {
      controller.enqueue(encoder.encode("x".repeat(5000)));
    },
    cancel() {
      cancelled = true;
    },
  });
  const { env, points } = environment();
  const response = await handle(
    new Request("https://telemetry.test/v1/events", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
      duplex: "half",
    }),
    env,
  );
  assert.equal(response.status, 413);
  assert.equal(points.length, 0);
  assert.equal(cancelled, true);
});

test("enforces the limit across several chunks and around a misleading length", async () => {
  const { env, points } = environment();
  const response = await handle(
    streamRequest(["a".repeat(1500), "b".repeat(1500)]),
    env,
  );
  assert.equal(response.status, 413);
  assert.equal(points.length, 0);
  // A small declared length must not let a larger stream through.
  const misleading = environment();
  const misleadResponse = await handle(
    streamRequest(["c".repeat(3000)], { "content-length": "10" }),
    misleading.env,
  );
  assert.equal(misleadResponse.status, 413);
  assert.equal(misleading.points.length, 0);
});

test("maps malformed and failed streamed bodies to a bounded error", async () => {
  const malformed = environment();
  assert.equal((await handle(streamRequest(["{not json"]), malformed.env)).status, 400);
  assert.equal(malformed.points.length, 0);
  const failing = new ReadableStream({
    start(controller) {
      controller.error(new Error("boom"));
    },
  });
  const failed = environment();
  const failedResponse = await handle(
    new Request("https://telemetry.test/v1/events", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: failing,
      duplex: "half",
    }),
    failed.env,
  );
  assert.equal(failedResponse.status, 400);
  assert.equal(failed.points.length, 0);
});
