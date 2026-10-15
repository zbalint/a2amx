import assert from "node:assert/strict";
import { mock, test } from "node:test";
import { enableTimers, makePi, startExtension, startServer } from "./harness.mjs";

async function connected(server, options = {}) {
  const h = await startExtension({ server, ...options });
  const connection = await server.waitForConnection(1);
  await server.waitForFrame(connection, (frame) => frame.type === "hello");
  return { h, connection };
}

async function ready(server, options = {}) {
  const result = await connected(server, options);
  server.send(result.connection, { type: "ready" });
  await server.waitForFrame(result.connection, (frame) => frame.type === "state");
  return result;
}

// shortcut: the fixed real-time sleeps below wait for effects that send no frame (a refusal is final
// and silent); replace them if the extension gains an observable signal for "refusal handled".
test("12 refusals notify final reasons, while duplicate attachment retries", async (t) => {
  for (const [reason, pattern] of [
    ["protocol_mismatch", /disagree on the protocol version/],
    ["missing_apis: isIdle", /isIdle/],
    ["session is not an omp session", /not started with a2amx/],
  ]) {
    const server = await startServer();
    const { h, connection } = await connected(server);
    t.after(async () => h.teardown());
    server.send(connection, { type: "ready" });
    await server.waitForFrame(connection, (frame) => frame.type === "state");
    const beforeRefusal = connection.frames.length;
    server.send(connection, { type: "refused", reason });
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert.equal(h.ctx.notifications.length, 1);
    assert.match(h.ctx.notifications[0].message, pattern);
    h.ctx.idle = false;
    await new Promise((resolve) => setTimeout(resolve, 700));
    assert.equal(connection.frames.length, beforeRefusal);
    server.close(connection);
    await h.waitForBridgeExit(1);
    await new Promise((resolve) => setTimeout(resolve, 350));
    assert.equal(server.connections.length, 1);
  }

  const unknownServer = await startServer();
  const unknown = await connected(unknownServer);
  t.after(async () => unknown.h.teardown());
  for (const reason of ["daemon said no", "daemon said later"]) {
    unknownServer.send(unknown.connection, { type: "refused", reason });
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.deepEqual(unknown.h.ctx.notifications.map((notice) => notice.message), ["daemon said no", "daemon said later"]);
  unknownServer.close(unknown.connection);
  await unknown.h.waitForBridgeExit(1);
  await new Promise((resolve) => setTimeout(resolve, 350));
  assert.equal(unknownServer.connections.length, 1);

  enableTimers(mock);
  const duplicateServer = await startServer();
  const duplicate = await connected(duplicateServer);
  t.after(async () => duplicate.h.teardown());
  duplicateServer.send(duplicate.connection, { type: "refused", reason: "a bridge is already attached to this session" });
  duplicateServer.close(duplicate.connection);
  await duplicate.h.waitForBridgeExit(1);
  mock.timers.tick(299);
  assert.equal(duplicateServer.connections.length, 1);
  mock.timers.tick(1);
  await duplicateServer.waitForConnection(2);
  assert.equal(duplicate.h.ctx.notifications.length, 0);
});

test("13 respawn backoff doubles through 5000ms and ready resets to 300ms", async (t) => {
  enableTimers(mock);
  const server = await startServer();
  const { h } = await connected(server);
  t.after(async () => h.teardown());
  for (const [number, delay] of [[1, 300], [2, 600], [3, 1200], [4, 2400], [5, 4800], [6, 5000]]) {
    const connection = server.connections[number - 1];
    server.close(connection);
    await h.waitForBridgeExit(number);
    mock.timers.tick(delay - 1);
    assert.equal(server.connections.length, number);
    mock.timers.tick(1);
    await server.waitForConnection(number + 1);
  }
  const resetConnection = server.connections[6];
  server.send(resetConnection, { type: "ready" });
  await server.waitForFrame(resetConnection, (frame) => frame.type === "state");
  server.close(resetConnection);
  await h.waitForBridgeExit(7);
  mock.timers.tick(299);
  assert.equal(server.connections.length, 7);
  mock.timers.tick(1);
  await server.waitForConnection(8);
});

test("14 tenth failed start warns once and continues respawning", async (t) => {
  enableTimers(mock);
  const server = await startServer();
  const { h } = await connected(server);
  t.after(async () => h.teardown());
  const delays = [300, 600, 1200, 2400, 4800, 5000, 5000, 5000, 5000, 5000, 5000];
  for (let number = 1; number <= 11; number += 1) {
    server.close(server.connections[number - 1]);
    await h.waitForBridgeExit(number);
    mock.timers.tick(delays[number - 1]);
    await server.waitForConnection(number + 1);
    if (number === 9) assert.equal(h.ctx.notifications.some((notice) => notice.message.includes("cannot reach its daemon")), false);
    if (number === 10) assert.equal(h.ctx.notifications.filter((notice) => notice.message.includes("cannot reach its daemon")).length, 1);
  }
  assert.equal(h.ctx.notifications.filter((notice) => notice.message.includes("cannot reach its daemon")).length, 1);
});

test("15 shutdown closes stdin and kills an ignored bridge after 1000ms", async (t) => {
  enableTimers(mock);
  const server = await startServer();
  const { h, connection } = await connected(server, { env: { FAKE_BRIDGE_IGNORE_STDIN_END: "1" } });
  t.after(async () => h.teardown());
  await h.pi.emit("session_shutdown", {}, h.ctx);
  mock.timers.tick(999);
  assert.equal(connection.socket.destroyed, false);
  mock.timers.tick(1);
  await h.waitForBridgeExit(1);
  assert.equal(connection.socket.destroyed, true);
  mock.timers.tick(5000);
  assert.equal(server.connections.length, 1);
});

test("16 registered tools relay JSON-RPC calls and decode all response forms", async (t) => {
  const server = await startServer();
  const { h, connection } = await ready(server);
  t.after(async () => h.teardown());
  const tool = h.pi.tools.find((candidate) => candidate.name === "message_status");
  const first = tool.execute("call-1", { i: "intent", id: "msg-1" });
  const request = await server.waitForFrame(connection, (frame) => frame.type === "mcp");
  assert.deepEqual(request.request, { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "message_status", arguments: { id: "msg-1" } } });
  server.send(connection, { type: "mcp", response: { id: 1, result: { content: [{ type: "text", text: "ok" }] } } });
  assert.deepEqual(await first, { content: [{ type: "text", text: "ok" }] });

  const errorResult = tool.execute("call-2", { id: "msg-2" });
  await server.waitForFrame(connection, (frame) => frame.type === "mcp" && frame.request.id === 2);
  server.send(connection, { type: "mcp", response: { id: 2, result: { isError: true, content: [{ text: "tool failed" }] } } });
  await assert.rejects(errorResult, { message: "tool failed" });

  const rpcError = tool.execute("call-3", { id: "msg-3" });
  await server.waitForFrame(connection, (frame) => frame.type === "mcp" && frame.request.id === 3);
  server.send(connection, { type: "mcp", response: { id: 3, error: { message: "rpc failed" } } });
  await assert.rejects(rpcError, { message: "rpc failed" });
});

test("17 tools throw before ready and reject on disconnect or timeout", async (t) => {
  const earlyServer = await startServer();
  const early = await connected(earlyServer);
  const tool = early.h.pi.tools.find((candidate) => candidate.name === "list_agents");
  assert.throws(() => tool.execute("early", {}), { message: "A2AMX is not connected" });
  earlyServer.send(early.connection, { type: "ready" });
  await earlyServer.waitForFrame(early.connection, (frame) => frame.type === "state");
  const disconnectPromise = tool.execute("disconnect", {});
  await earlyServer.waitForFrame(early.connection, (frame) => frame.type === "mcp");
  earlyServer.close(early.connection);
  await assert.rejects(disconnectPromise, { message: "A2AMX connection lost" });
  await early.h.teardown();

  enableTimers(mock);
  const timeoutServer = await startServer();
  const timeout = await ready(timeoutServer);
  t.after(async () => timeout.h.teardown());
  const pending = timeout.h.pi.tools.find((candidate) => candidate.name === "list_agents").execute("timeout", {});
  await timeoutServer.waitForFrame(timeout.connection, (frame) => frame.type === "mcp");
  mock.timers.tick(59999);
  let settled = false;
  pending.catch(() => { settled = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false);
  mock.timers.tick(1);
  await assert.rejects(pending, { message: "A2AMX did not answer in time" });
});
