const MAX_BODY = 2048;

const KEYS_V1 = [
  "v", "event", "release", "os", "arch", "shell", "mode", "route", "decision",
  "effects", "proposal_outcome", "execution_outcome", "user_feedback", "latency",
  "cache", "interactive", "notice_revision",
];
const KEYS_V2 = [...KEYS_V1, "parent_action"];

const ENUMS = Object.freeze({
  event: ["interaction_summary", "feedback_summary"],
  os: ["linux", "macos", "other"],
  arch: ["x86_64", "aarch64", "other"],
  shell: ["sh", "bash", "zsh", "fish", "pwsh", "powershell", "other"],
  mode: ["auto", "run", "ask", "explain"],
  route: ["unknown", "answer", "shell", "program", "parent_shell", "clarification"],
  decision: ["not_run", "ran", "returned", "dry_run", "cancelled", "needs_parent", "unavailable"],
  effects: ["none", "read_local", "write_local", "delete_local", "network_read", "remote_mutation", "privilege_elevation", "process_control", "shell_state", "unknown"],
  proposal_outcome: ["not_requested", "valid", "invalid", "refused", "incomplete"],
  execution_outcome: ["not_run", "exit_zero", "exit_nonzero", "signal", "timeout", "spawn_error", "output_overflow"],
  user_feedback: ["unknown", "good", "bad"],
  latency: ["lt_1s", "1s_2s", "2s_5s", "gte_5s"],
  cache: ["unknown", "miss", "hit", "disabled"],
  parent_action: ["not_applicable", "unknown", "applied", "failed"],
});

export function validateEvent(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const keys = Object.keys(value).sort();
  const expected = value.v === 1 ? KEYS_V1 : value.v === 2 ? KEYS_V2 : [];
  if (keys.length !== expected.length || keys.some((key, index) => key !== [...expected].sort()[index])) return false;
  if ((value.v === 1 && ![1, 2].includes(value.notice_revision)) || (value.v === 2 && value.notice_revision !== 3) || typeof value.interactive !== "boolean") return false;
  if (typeof value.release !== "string" || !/^\d+\.\d+$/.test(value.release)) return false;
  return Object.entries(ENUMS).filter(([key]) => key !== "parent_action" || value.v === 2).every(([key, allowed]) =>
    typeof value[key] === "string" && allowed.includes(value[key]),
  );
}

export function eventToPoint(event) {
  return {
    indexes: [event.event],
    blobs: [
      event.release, event.os, event.arch, event.shell, event.mode, event.route, event.decision,
      event.effects, event.proposal_outcome, event.execution_outcome, event.user_feedback,
      event.latency, event.cache, event.parent_action || "not_applicable",
    ],
    doubles: [event.interactive ? 1 : 0, event.notice_revision, event.v],
  };
}

function response(status) {
  return new Response(null, {
    status,
    headers: { "cache-control": "no-store", "content-type": "text/plain; charset=utf-8" },
  });
}

export async function handle(request, env) {
  if (request.method !== "POST" || new URL(request.url).pathname !== "/v1/events") return response(404);
  if (env.ENABLED !== "true") return response(503);
  if (request.headers.get("content-type")?.split(";", 1)[0].trim().toLowerCase() !== "application/json") return response(415);
  const declared = Number(request.headers.get("content-length") || 0);
  if (declared >= MAX_BODY) return response(413);

  const allowed = await env.RATE_LIMITER.limit({ key: "events-v1" });
  if (!allowed.success) return response(429);

  const bytes = await request.arrayBuffer();
  if (bytes.byteLength >= MAX_BODY) return response(413);
  let event;
  try {
    event = JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    return response(400);
  }
  if (!validateEvent(event)) return response(422);
  env.EVENTS.writeDataPoint(eventToPoint(event));
  return response(202);
}

export default { fetch: handle };
