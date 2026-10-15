import assert from "node:assert/strict";
import { mock, test } from "node:test";
import {
  enableTimers, makeCtx, makePi, startExtension, startServer,
} from "./harness.mjs";

async function ready(h, server) {
  const connection = await server.waitForConnection(1);
  const hello = await server.waitForFrame(connection, (frame) => frame.type === "hello");
  server.send(connection, { type: "ready" });
  await server.waitForFrame(connection, (frame) => frame.type === "state");
  return { connection, hello };
}

test("1 startup gates non-main sessions and reports each missing environment once", async (t) => {
  for (const missing of ["A2AMX_BIN", "A2AMX_ADDR", "A2AMX_TOKEN"]) {
    const server = await startServer();
    const ctx = makeCtx({ kind: "main" });
    const h = await startExtension({ server, ctx, env: { [missing]: undefined } });
    t.after(async () => h.teardown());
    assert.equal(server.connections.length, 0);
    assert.equal(ctx.notifications.length, 1);
    assert.equal(ctx.notifications[0].level, "error");
    assert.match(ctx.notifications[0].message, new RegExp(`${missing} is not set`));
    await h.pi.emit("session_start", {}, ctx);
    assert.equal(ctx.notifications.length, 1);
  }
  const server = await startServer();
  const ctx = makeCtx({ kind: "sub" });
  const h = await startExtension({ server, ctx });
  t.after(async () => h.teardown());
  assert.equal(server.connections.length, 0);
  assert.equal(ctx.notifications.length, 0);
});

test("2 hello reports protocol, OMP version and missing APIs", async (t) => {
  const server = await startServer();
  const ctx = makeCtx({ missing: ["isIdle"] });
  const pi = makePi({ version: "test-version" });
  const h = await startExtension({ server, ctx, pi });
  t.after(async () => h.teardown());
  const connection = await server.waitForConnection(1);
  assert.deepEqual(await server.waitForFrame(connection, (frame) => frame.type === "hello"), {
    type: "hello", protocol: 1, omp_version: "test-version", missing: ["isIdle"],
  });
  assert.equal(connection.frames.some((frame) => frame.type === "state"), false);
});

test("3 registers literal native tools and reports registration failures", async (t) => {
  const server = await startServer();
  const pi = makePi();
  const h = await startExtension({ server, pi });
  t.after(async () => h.teardown());
  assert.deepEqual(pi.tools.map((tool) => tool.name), ["list_agents", "send_message", "message_status", "reset_session"]);
  assert.equal(pi.tools.find((tool) => tool.name === "send_message").parameters.required[0], "to");
  await h.pi.emit("session_shutdown", {}, h.ctx);

  const failingServer = await startServer();
  const failingPi = makePi();
  failingPi.registerTool = (tool) => { throw new Error(`cannot register ${tool.name}`); };
  const failing = await startExtension({ server: failingServer, pi: failingPi });
  t.after(async () => failing.teardown());
  assert.equal(failing.ctx.notifications.length, 4);
  assert.deepEqual(failing.ctx.notifications.map((notice) => notice.message), [
    "cannot register list_agents", "cannot register send_message",
    "cannot register message_status", "cannot register reset_session",
  ]);
});

test("4 ready reports idle, pending, draft and host state literals", async (t) => {
  const server = await startServer();
  const ctx = makeCtx();
  const h = await startExtension({ server, ctx });
  t.after(async () => h.teardown());
  const { connection } = await ready(h, server);
  assert.deepEqual(connection.frames.at(-1), { type: "state", idle: true, pending: false, draft: false, queue: 0, host_pending: false });
  ctx.hasPending = true;
  const pendingState = server.waitForFrame(connection, (frame) => frame.type === "state" && frame.host_pending === true);
  await h.pi.emit("turn_start", {}, ctx);
  assert.deepEqual(await pendingState, { type: "state", idle: true, pending: true, draft: false, queue: 0, host_pending: true });
  ctx.hasPending = false;
  ctx.draft = "  ";
  const blankState = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.draft === false && frame.host_pending === false);
  await h.pi.emit("turn_end", {}, ctx);
  await blankState;
  ctx.draft = "hi";
  const draftState = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.draft === true);
  await h.pi.emit("agent_start", {}, ctx);
  await draftState;
  ctx.ui.getEditorText = () => { throw new Error("editor gone"); };
  const errorDraftState = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.draft === false);
  await h.pi.emit("agent_end", {}, ctx);
  await errorDraftState;
});

test("5 lifecycle events suppress unchanged state and send changed state", async (t) => {
  const server = await startServer();
  const h = await startExtension({ server });
  t.after(async () => h.teardown());
  const { connection } = await ready(h, server);
  const count = connection.frames.length;
  for (const event of ["turn_start", "turn_end", "agent_start", "agent_end"]) await h.pi.emit(event, {}, h.ctx);
  assert.equal(connection.frames.length, count);
  h.ctx.idle = false;
  const changed = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.idle === false);
  await h.pi.emit("turn_start", {}, h.ctx);
  await changed;
});

test("6 poll detects draft changes and keepalive repeats at 30000ms", async (t) => {
  enableTimers(mock);
  const server = await startServer();
  const h = await startExtension({ server });
  t.after(async () => h.teardown());
  const { connection } = await ready(h, server);
  const before = connection.frames.length;
  h.ctx.draft = "polled";
  mock.timers.tick(499);
  assert.equal(connection.frames.length, before);
  const polled = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.draft === true);
  mock.timers.tick(1);
  await polled;
  const keepaliveStart = connection.frames.length;
  mock.timers.tick(29999);
  assert.equal(connection.frames.length, keepaliveStart);
  const keepalive = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.draft === true);
  mock.timers.tick(1);
  await keepalive;
  assert.equal(connection.frames.length, keepaliveStart + 1);
});
