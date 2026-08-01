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
    notice_revision: 3,
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
  assert.deepEqual(points[0].doubles, [1, 3, 2]);
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
    event({ notice_revision: 4 }),
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
