import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { mock } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const realSetTimeout = globalThis.setTimeout;
const root = path.dirname(fileURLToPath(import.meta.url));
const fakeBridge = path.join(root, "fake_bridge.mjs");

// The module is imported once: the default export is a factory and keeps its state per call.
export async function loadExtension() {
  const filename = process.env.A2AMX_TEST_EXTENSION;
  if (!filename) throw new Error("A2AMX_TEST_EXTENSION is not set");
  const module = await import(pathToFileURL(filename).href);
  return module.default;
}

export function makePi({ version = "18.4.5", sendUserMessage } = {}) {
  const handlers = new Map();
  const calls = [];
  const tools = [];
  const pi = {
    pi: version === null ? {} : { VERSION: version },
    tools,
    calls,
    on(name, handler) { handlers.set(name, handler); },
    emit(name, event, ctx) { return handlers.get(name)?.(event, ctx); },
    sendUserMessage(text, options) {
      calls.push({ text, options });
      if (sendUserMessage) return sendUserMessage(text, options);
    },
    registerTool(tool) { tools.push(tool); },
  };
  return pi;
}

export function makeCtx(overrides = {}) {
  const notifications = [];
  const ctx = {
    idle: overrides.idle ?? true,
    hasPending: overrides.hasPending ?? false,
    draft: overrides.draft ?? "",
    notifications,
    isIdle: overrides.isIdle ?? function () { return this.idle; },
    hasPendingMessages: overrides.hasPendingMessages ?? function () { return this.hasPending; },
    ui: {
      getEditorText: overrides.getEditorText ?? (() => ctx.draft),
      notify(message, level) { notifications.push({ message, level }); },
    },
    agent: { kind: overrides.kind ?? "main" },
  };
  if (Object.hasOwn(overrides, "model")) ctx.model = overrides.model;
  if (overrides.missing) {
    for (const name of overrides.missing) {
      if (name === "getEditorText") delete ctx.ui.getEditorText;
      else delete ctx[name];
    }
  }
  return ctx;
}

function timeout(message) {
  return new Promise((_, reject) => realSetTimeout(() => reject(new Error(message)), 5000));
}

export async function startServer() {
  const server = net.createServer();
  const connections = [];
  server.on("connection", (socket) => {
    const connection = { socket, frames: [], waiters: [], buffer: "" };
    connections.push(connection);
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      connection.buffer += chunk;
      let end;
      while ((end = connection.buffer.indexOf("\n")) >= 0) {
        const line = connection.buffer.slice(0, end);
        connection.buffer = connection.buffer.slice(end + 1);
        if (!line) continue;
        connection.frames.push(JSON.parse(line));
        for (const waiter of [...connection.waiters]) {
          if (waiter.predicate(connection.frames.at(-1))) {
            connection.waiters.splice(connection.waiters.indexOf(waiter), 1);
            waiter.resolve(connection.frames.at(-1));
          }
        }
      }
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  return {
    connections,
    port: address.port,
    send(connection, frame) { connection.socket.write(`${JSON.stringify(frame)}\n`); },
    close(connection) { connection.socket.destroy(); },
    async waitForConnection(number) {
      const started = process.hrtime.bigint();
      while (Number(process.hrtime.bigint() - started) < 5_000_000_000) {
        const found = connections[number - 1];
        if (found) return found;
        await new Promise((resolve) => realSetTimeout(resolve, 5));
      }
      throw new Error(`timed out waiting for bridge connection ${number}`);
    },
    async waitForFrame(connection, predicate, start = 0) {
      const found = connection.frames.slice(start).find(predicate);
      if (found) return found;
      return Promise.race([new Promise((resolve) => connection.waiters.push({ predicate, resolve })), timeout("timed out waiting for bridge frame")]);
    },
    async waitForNewFrame(connection, predicate, start = connection.frames.length) {
      return this.waitForFrame(connection, predicate, start);
    },
    async shutdown() {
      for (const connection of connections) connection.socket.destroy();
      await Promise.race([
        new Promise((resolve) => server.close(resolve)),
        timeout("timed out closing test server"),
      ]);
    },
  };
}

export async function startExtension({ server, env = {}, tools = true, pi, ctx } = {}) {
  const previous = new Map();
  const pidDir = fs.mkdtempSync(path.join(os.tmpdir(), "a2amx-omp-pids-"));
  const values = {
    A2AMX_BIN: fakeBridge,
    A2AMX_ADDR: "127.0.0.1:1",
    A2AMX_TOKEN: "test-token",
    FAKE_BRIDGE_PORT: String(server.port),
    FAKE_BRIDGE_PID_DIR: pidDir,
    ...env,
  };
  for (const [name, value] of Object.entries(values)) {
    previous.set(name, process.env[name]);
    if (value === undefined) delete process.env[name]; else process.env[name] = value;
  }
  const extension = await loadExtension();
  const actualPi = pi ?? makePi();
  const actualCtx = ctx ?? makeCtx();
  extension(actualPi);
  await actualPi.emit("session_start", {}, actualCtx);
  return {
    pi: actualPi,
    ctx: actualCtx,
    pidDir,
    async waitForBridgeExit(number) {
      const started = process.hrtime.bigint();
      while (Number(process.hrtime.bigint() - started) < 5_000_000_000) {
        const files = fs.readdirSync(pidDir).map(Number).filter(Number.isFinite).sort((a, b) => a - b);
        if (files.length >= number) {
          const pid = files[number - 1];
          try { process.kill(pid, 0); } catch (error) {
            if (error.code === "ESRCH") {
              await new Promise((resolve) => realSetTimeout(resolve, 100)); // shortcut: fixed settle delay, no signal for "close handled" in the extension; replace if the extension gains one
              return;
            }
          }
        }
        await new Promise((resolve) => realSetTimeout(resolve, 10));
      }
      throw new Error(`timed out waiting for bridge ${number} exit`);
    },
    async teardown() {
      let failure;
      try {
        await actualPi.emit("session_shutdown", {}, actualCtx);
      } catch (error) {
        failure = error;
      }
      if (server.connections.length) {
        try { await this.waitForBridgeExit(server.connections.length); } catch (error) { failure ??= error; }
      }
      try { mockTimersReset(); } catch (error) { failure ??= error; }
      try { await server.shutdown(); } catch (error) { failure ??= error; }
      for (const [name, value] of previous) {
        if (value === undefined) delete process.env[name]; else process.env[name] = value;
      }
      fs.rmSync(pidDir, { recursive: true, force: true });
      if (failure) throw failure;
    },
  };
}

function mockTimersReset() {
  mock.timers.reset();
}

export function enableTimers(mock) {
  mock.timers.enable({ apis: ["setTimeout", "setInterval", "Date"] });
}
