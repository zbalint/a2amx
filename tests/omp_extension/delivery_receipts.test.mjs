import assert from "node:assert/strict";
import { mock, test } from "node:test";
import { enableTimers, makePi, startExtension, startServer } from "./harness.mjs";

async function ready(server, options) {
  const h = await startExtension({ server, ...options });
  const connection = await server.waitForConnection(1);
  await server.waitForFrame(connection, (frame) => frame.type === "hello");
  server.send(connection, { type: "ready" });
  await server.waitForFrame(connection, (frame) => frame.type === "state");
  return { h, connection };
}

async function deliver(server, connection, id, envelope) {
  server.send(connection, { type: "deliver", id, envelope });
  return server.waitForFrame(connection, (frame) => (frame.type === "ack" || frame.type === "nack") && frame.id === id);
}
test("7 deliver uses idle and aside options and acknowledges with queue state", async (t) => {
  const server = await startServer();
  const { h, connection } = await ready(server);
  t.after(async () => h.teardown());
  const envelope = "<a2amx-message>idle</a2amx-message>";
  const ack = await deliver(server, connection, "m1", envelope);
  assert.deepEqual(ack, { type: "ack", id: "m1" });
  assert.deepEqual(h.pi.calls.at(-1), { text: envelope, options: { attribution: "agent" } });
  const state = await server.waitForFrame(connection, (frame) => frame.type === "state" && frame.queue === 1);
  assert.ok(connection.frames.indexOf(ack) < connection.frames.indexOf(state));
  h.ctx.idle = false;
  const envelope2 = "<a2amx-message>busy</a2amx-message>";
  await deliver(server, connection, "m2", envelope2);
  assert.deepEqual(h.pi.calls.at(-1), { text: envelope2, options: { deliverAs: "aside", attribution: "agent" } });
});

test("8 message_end accepts string and text-part receipts and rejects mismatches", async (t) => {
  const server = await startServer();
  const { h, connection } = await ready(server);
  t.after(async () => h.teardown());
  const first = "<a2amx-message>one</a2amx-message>";
  await deliver(server, connection, "m1", first);
  const firstStart = connection.frames.length;
  const firstReceipt = server.waitForFrame(connection, (frame) => frame.type === "receipt" && frame.id === "m1");
  const firstState = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.queue === 0, firstStart);
  await h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: first } }, h.ctx);
  const firstReceiptFrame = await firstReceipt;
  const firstStateFrame = await firstState;
  assert.ok(connection.frames.indexOf(firstReceiptFrame) < connection.frames.indexOf(firstStateFrame));

  const second = "<a2amx-message>two</a2amx-message>";
  await deliver(server, connection, "m2", second);
  const secondStart = connection.frames.length;
  const secondReceipt = server.waitForFrame(connection, (frame) => frame.type === "receipt" && frame.id === "m2");
  const secondState = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.queue === 0, secondStart);
  await h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: [{ type: "text", text: "<a2amx-message>" }, { type: "text", text: "two</a2amx-message>" }] } }, h.ctx);
  const secondReceiptFrame = await secondReceipt;
  const secondStateFrame = await secondState;
  assert.ok(connection.frames.indexOf(secondReceiptFrame) < connection.frames.indexOf(secondStateFrame));

  const pending = "<a2amx-message>pending</a2amx-message>";
  await deliver(server, connection, "m3", pending);
  const negativeStart = connection.frames.length;
  for (const message of [
    { role: "assistant", attribution: "agent", content: pending },
    { role: "user", attribution: "human", content: pending },
    { role: "user", attribution: "agent", content: "other" },
  ]) {
    await h.pi.emit("message_end", { message }, h.ctx);
  }
  h.ctx.draft = "negative-barrier";
  const negativeBarrier = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.draft === true, negativeStart);
  await h.pi.emit("agent_start", {}, h.ctx);
  await negativeBarrier;
  assert.equal(connection.frames.slice(negativeStart).some((frame) => frame.type === "receipt"), false);
  const pendingStart = connection.frames.length;
  const pendingReceipt = server.waitForFrame(connection, (frame) => frame.type === "receipt" && frame.id === "m3");
  const pendingState = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.queue === 0, pendingStart);
  await h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: pending } }, h.ctx);
  const pendingReceiptFrame = await pendingReceipt;
  const pendingStateFrame = await pendingState;
  assert.ok(connection.frames.indexOf(pendingReceiptFrame) < connection.frames.indexOf(pendingStateFrame));
});

test("9 receipts are FIFO, out-of-order events are forgotten, and sync events defer", async (t) => {
  const server = await startServer();
  const { h, connection } = await ready(server);
  t.after(async () => h.teardown());
  const one = "<a2amx-message>one</a2amx-message>";
  const two = "<a2amx-message>two</a2amx-message>";
  await deliver(server, connection, "m1", one);
  await deliver(server, connection, "m2", two);
  const negativeStart = connection.frames.length;
  await h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: two } }, h.ctx);
  h.ctx.idle = false;
  const negativeBarrier = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.idle === false, negativeStart);
  await h.pi.emit("turn_start", {}, h.ctx);
  await negativeBarrier;
  assert.equal(connection.frames.slice(negativeStart).some((frame) => frame.type === "receipt"), false);
  const firstReceipt = server.waitForFrame(connection, (frame) => frame.type === "receipt" && frame.id === "m1");
  await h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: one } }, h.ctx);
  await firstReceipt;
  const secondStart = connection.frames.length;
  const secondReceipt = server.waitForNewFrame(connection, (frame) => frame.type === "receipt" && frame.id === "m2", secondStart);
  await h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: two } }, h.ctx);
  await secondReceipt;

  const syncServer = await startServer();
  const pi = makePi();
  const sync = await ready(syncServer, { pi });
  t.after(async () => sync.h.teardown());
  const syncEnvelope = "<a2amx-message>sync</a2amx-message>";
  sync.h.pi.sendUserMessage = (text, options) => {
    sync.h.pi.calls.push({ text, options });
    sync.h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: text } }, sync.h.ctx);
  };
  const syncReceipt = syncServer.waitForFrame(sync.connection, (frame) => frame.type === "receipt" && frame.id === "sync");
  await deliver(syncServer, sync.connection, "sync", syncEnvelope);
  await syncReceipt;
  assert.ok(sync.connection.frames.findIndex((frame) => frame.type === "ack") < sync.connection.frames.findIndex((frame) => frame.type === "receipt"));
});

test("10 send failure nacks with a 200-character reason and leaves FIFO empty", async (t) => {
  const server = await startServer();
  const pi = makePi({ sendUserMessage: () => { throw new Error("x".repeat(300)); } });
  const { h, connection } = await ready(server, { pi });
  t.after(async () => h.teardown());
  const nack = await deliver(server, connection, "bad", "<a2amx-message>bad</a2amx-message>");
  assert.equal(nack.type, "nack");
  assert.equal(nack.reason.length, 200);
  assert.equal(connection.frames.some((frame) => frame.type === "ack"), false);
  h.pi.sendUserMessage = () => undefined;
  const ack = await deliver(server, connection, "good", "<a2amx-message>good</a2amx-message>");
  assert.deepEqual(ack, { type: "ack", id: "good" });
  await server.waitForFrame(connection, (frame) => frame.type === "state" && frame.queue === 1);
});

test("11 late receipts honor 120000ms grace and stale heads expire", async (t) => {
  enableTimers(mock);
  const server = await startServer();
  const { h, connection } = await ready(server);
  t.after(async () => h.teardown());
  const envelope = "<a2amx-message>late</a2amx-message>";
  await deliver(server, connection, "late", envelope);
  h.ctx.hasPending = true;
  await h.pi.emit("turn_start", {}, h.ctx);
  h.ctx.hasPending = false;
  const dropped = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.queue === 0);
  await h.pi.emit("turn_end", {}, h.ctx);
  await dropped;
  mock.timers.tick(119999);
  const lateReceipt = server.waitForFrame(connection, (frame) => frame.type === "receipt" && frame.id === "late");
  await h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: envelope } }, h.ctx);
  await lateReceipt;

  const expired = "<a2amx-message>expired</a2amx-message>";
  await deliver(server, connection, "expired", expired);
  h.ctx.hasPending = true;
  await h.pi.emit("turn_start", {}, h.ctx);
  h.ctx.hasPending = false;
  const expiredDropped = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.queue === 0);
  await h.pi.emit("turn_end", {}, h.ctx);
  await expiredDropped;
  mock.timers.tick(120000);
  const expiredStart = connection.frames.length;
  await h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: expired } }, h.ctx);
  h.ctx.draft = "expired-barrier";
  const expiredBarrier = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.draft === true, expiredStart);
  await h.pi.emit("agent_start", {}, h.ctx);
  await expiredBarrier;
  assert.equal(connection.frames.slice(expiredStart).some((frame) => frame.type === "receipt" && frame.id === "expired"), false);

  const stale = "<a2amx-message>stale</a2amx-message>";
  await deliver(server, connection, "stale", stale);
  mock.timers.tick(120001);
  const staleStart = connection.frames.length;
  h.ctx.idle = false;
  const staleState = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.queue === 0, staleStart);
  await h.pi.emit("agent_start", {}, h.ctx);
  await staleState;
  mock.timers.tick(120000);
  const before = connection.frames.length;
  await h.pi.emit("message_end", { message: { role: "user", attribution: "agent", content: stale } }, h.ctx);
  h.ctx.idle = true;
  const staleBarrier = server.waitForNewFrame(connection, (frame) => frame.type === "state" && frame.idle === true, before);
  await h.pi.emit("agent_end", {}, h.ctx);
  await staleBarrier;
  assert.equal(connection.frames.slice(before).some((frame) => frame.type === "receipt"), false);
});
