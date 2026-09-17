#!/usr/bin/env node
// ABOUTME: Checks the real uhm binary's telemetry payload against the local Worker gateway in-process.
// ABOUTME: Never sends HTTP to the live endpoint; the Worker handle runs with mocked bindings.

import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { handle } from "../telemetry-worker/src/index.js";

const argument = process.argv[2];
const cli = argument ? path.resolve(argument) : "";
if (process.argv.length !== 3 || !existsSync(cli)) {
  console.error("usage: node scripts/test-telemetry-contract.mjs <path-to-uhm-binary>");
  process.exit(2);
}

const home = mkdtempSync(path.join(tmpdir(), "uhm-telemetry-contract-"));
let failures = 0;
const check = (ok, message) => {
  if (!ok) {
    failures += 1;
    console.error(`FAIL: ${message}`);
  }
};

// The exact key set a released expanded-v2 client may send. Duplicated here
// on purpose: this script is the cross-check, not a mirror of Worker tables.
const RELEASED_EXPANDED_V2_KEYS = [
  "v", "event", "release", "os", "arch", "shell", "mode", "route", "decision",
  "effects", "proposal_outcome", "execution_outcome", "user_feedback", "latency",
  "cache", "interactive", "notice_revision", "parent_action", "expansion_outcome",
];

try {
  const run = spawnSync(cli, ["telemetry", "preview"], {
    encoding: "utf8",
    env: {
      PATH: process.env.PATH ?? "/usr/bin:/bin",
      HOME: home,
      XDG_CONFIG_HOME: path.join(home, "config"),
      XDG_DATA_HOME: path.join(home, "data"),
      XDG_CACHE_HOME: path.join(home, "cache"),
      TERM: "dumb",
      DO_NOT_TRACK: "1",
    },
  });
  check(
    run.status === 0,
    `telemetry preview exited with ${run.status}: ${run.stderr}`,
  );
  const event = JSON.parse(run.stdout);

  const unexpected = Object.keys(event).filter(
    (key) => !RELEASED_EXPANDED_V2_KEYS.includes(key),
  );
  check(
    unexpected.length === 0,
    `payload carries fields outside the released key set: ${unexpected.join(", ")}`,
  );

  const points = [];
  const response = await handle(
    new Request("https://telemetry.test/v1/events", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(event),
    }),
    {
      ENABLED: "true",
      RATE_LIMITER: { limit: async () => ({ success: true }) },
      EVENTS: { writeDataPoint: (point) => points.push(point) },
    },
  );
  check(
    response.status === 202,
    `the local Worker rejected the real CLI payload with ${response.status}`,
  );
  check(points.length === 1, `expected exactly one point, got ${points.length}`);
  if (points.length === 1) {
    const [point] = points;
    check(
      point.indexes.length === 1 && point.indexes[0] === "interaction_summary",
      `unexpected indexes: ${JSON.stringify(point.indexes)}`,
    );
    check(point.blobs[0] === event.release, "blob1 must stay the release");
    check(point.blobs[3] === event.shell, "blob4 must stay the shell");
    check(point.blobs[13] === event.parent_action, "blob14 must stay parent_action");
    check(
      point.blobs[14] === event.expansion_outcome,
      `blob15 must carry expansion_outcome, got ${point.blobs[14]}`,
    );
    check(
      point.doubles.length === 3 &&
        point.doubles[0] === (event.interactive ? 1 : 0) &&
        point.doubles[1] === event.notice_revision &&
        point.doubles[2] === event.v,
      `unexpected doubles: ${JSON.stringify(point.doubles)}`,
    );
  }
} finally {
  rmSync(home, { recursive: true, force: true });
}

if (failures > 0) {
  console.error(`telemetry contract: ${failures} failure(s)`);
  process.exit(1);
}
console.log("telemetry contract: ok");
