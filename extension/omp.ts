import { spawn } from "node:child_process";

const PROTOCOL = 1;
const TOOLS = [/* a2amx:tools */];

export default function (pi) {
  let ctx = null;
  let initialized = false;
  let shuttingDown = false;
  let bridge = null;
  let bridgeReady = false;
  let respawnTimer = null;
  let pollTimer = null;
  let lastState = null;
  let backoff = 300;
  let failedStarts = 0;
  let lastStderrLine = "";
  let nextRequestId = 1;
  let delivering = false;
  let deferredReceipts = [];
  let drainingReceipts = false;
  const pendingCalls = new Map();
  const fifo = [];
  const late = [];
  const notified = new Set();

  function isMainSession() {
    return ctx?.agent?.kind === "main";
  }

  function notifyOnce(reason, message) {
    if (notified.has(reason)) return;
    notified.add(reason);
    ctx.ui.notify(message, "error");
  }

  function missingApis(sessionContext) {
    const missing = [];
    if (typeof pi?.sendUserMessage !== "function") missing.push("sendUserMessage");
    if (typeof pi?.registerTool !== "function") missing.push("registerTool");
    if (typeof sessionContext?.isIdle !== "function") missing.push("isIdle");
    if (typeof sessionContext?.hasPendingMessages !== "function") {
      missing.push("hasPendingMessages");
    }
    if (typeof sessionContext?.ui?.getEditorText !== "function") {
      missing.push("getEditorText");
    }
    return missing;
  }

  function safeDraft() {
    if (typeof ctx?.ui?.getEditorText !== "function") return false;
    try {
      const text = ctx.ui.getEditorText();
      return typeof text === "string" && text.trim() !== "";
    } catch {
      return false;
    }
  }

  function isCurrentBridge(state) {
    return state !== null && bridge === state && !state.finished && !state.processError;
  }

  function sendFrame(state, frame) {
    if (!isCurrentBridge(state) || !state.child.stdin) return false;
    try {
      state.child.stdin.write(`${JSON.stringify(frame)}\n`);
      return true;
    } catch (error) {
      recordBridgeError(state, error);
      return false;
    }
  }

  function stopPoll() {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  function startPoll() {
    if (pollTimer !== null || !bridgeReady) return;
    // shortcut: OMP has no editor-change event; use one if it gains it.
    pollTimer = setInterval(() => {
      if (bridgeReady) reportState(false);
    }, 500);
    pollTimer.unref?.();
  }

  function rejectPendingCalls() {
    for (const [id, call] of pendingCalls) {
      pendingCalls.delete(id);
      clearTimeout(call.timer);
      call.reject(new Error("A2AMX connection lost"));
    }
  }

  function firstContentText(content) {
    const first = Array.isArray(content) ? content[0] : undefined;
    if (typeof first === "string") return first;
    if (typeof first?.text === "string") return first.text;
    return String(first ?? "");
  }

  function handleMcpResponse(response) {
    if (!response) return;
    const id = response.id;
    const call = pendingCalls.get(id);
    if (!call) return;
    pendingCalls.delete(id);
    clearTimeout(call.timer);
    if (response.error) {
      call.reject(new Error(String(response.error?.message ?? response.error)));
      return;
    }
    const result = response.result;
    if (result?.isError === true) {
      call.reject(new Error(firstContentText(result.content)));
      return;
    }
    call.resolve({ content: result?.content });
  }

  function sendReceipt(id) {
    const state = bridge;
    if (state) sendFrame(state, { type: "receipt", id });
  }

  function receiptText(message) {
    if (!message || message.role !== "user" || message.attribution !== "agent") {
      return null;
    }
    if (typeof message.content === "string") return message.content;
    if (!Array.isArray(message.content)) return null;
    let text = "";
    for (const part of message.content) {
      if (part?.type === "text" && typeof part.text === "string") {
        text += part.text;
      }
    }
    return text;
  }

  function drainDeferredReceipts() {
    if (delivering || drainingReceipts) return;
    drainingReceipts = true;
    try {
      while (fifo.length > 0 && deferredReceipts.length > 0) {
        const index = deferredReceipts.indexOf(fifo[0].envelope);
        if (index < 0) break;
        const entry = fifo.shift();
        deferredReceipts.splice(index, 1);
        sendReceipt(entry.id);
        reportState(false);
      }
    } finally {
      drainingReceipts = false;
    }
  }

  function handleReceiptText(text) {
    if (text === null) return;
    if (fifo.length === 0 || text !== fifo[0].envelope) {
      const index = late.findIndex(
        (entry) => entry.envelope === text && Date.now() - entry.droppedAt < 120000,
      );
      if (index >= 0) {
        const [entry] = late.splice(index, 1);
        sendReceipt(entry.id);
        reportState(false);
        return;
      }
    }
    if (delivering) {
      if (fifo.length > 0 && text === fifo[0].envelope) {
        deferredReceipts.push(text);
      } else if (fifo.length === 0 && delivering.envelope === text) {
        deferredReceipts.push(text);
      }
      return;
    }
    if (fifo.length === 0 || text !== fifo[0].envelope) return;
    const entry = fifo.shift();
    sendReceipt(entry.id);
    reportState(false);
    drainDeferredReceipts();
  }

  function handleMessageEnd(event) {
    if (!isMainSession()) return;
    handleReceiptText(receiptText(event?.message));
  }

  function dropHeadForLateReceipt(now) {
    const entry = fifo.shift();
    late.push({ id: entry.id, envelope: entry.envelope, droppedAt: now });
    drainDeferredReceipts();
  }

  function reportState(force) {
    const now = Date.now();
    while (late.length > 0 && now - late[0].droppedAt >= 120000) late.shift();
    const state = bridge;
    if (!bridgeReady || !isCurrentBridge(state) || (delivering && !force)) return;

    const idle = ctx.isIdle();
    const hasPending = ctx.hasPendingMessages();
    if (hasPending && fifo.length > 0) fifo[0].sawPending = true;
    if (!delivering && !hasPending && fifo.length > 0 && fifo[0].sawPending) {
      dropHeadForLateReceipt(now);
    }
    // shortcut: fixed 120 s grace for changed message_end events; late receipts share that ceiling.
    while (!delivering && !hasPending && fifo.length > 0 && now - fifo[0].at > 120000) {
      dropHeadForLateReceipt(now);
    }

    const value = { idle, pending: fifo.length > 0 || hasPending, draft: safeDraft() };
    if (
      !force &&
      lastState &&
      lastState.idle === value.idle &&
      lastState.pending === value.pending &&
      lastState.draft === value.draft
    ) {
      return;
    }
    if (!sendFrame(state, { type: "state", ...value })) return;
    lastState = value;
  }

  function handleDeliver(frame) {
    if (!isMainSession()) return;
    const envelope = frame.envelope;
    const idle = ctx.isIdle();
    delivering = { envelope, id: frame.id };
    try {
      pi.sendUserMessage(
        envelope,
        // aside: lands at the next step boundary without interrupting the tool batch. followUp
        // waits for the turn to end, which a long goal-style turn never does, and a user
        // interrupt then clears it.
        idle ? { attribution: "agent" } : { deliverAs: "aside", attribution: "agent" },
      );
    } catch (error) {
      delivering = false;
      deferredReceipts = deferredReceipts.filter((text) =>
        fifo.some((entry) => entry.envelope === text),
      );
      const reason = String(error?.message ?? error).slice(0, 200);
      sendFrame(bridge, { type: "nack", id: frame.id, reason });
      drainDeferredReceipts();
      return;
    }
    fifo.push({ id: frame.id, envelope, at: Date.now(), sawPending: false });
    sendFrame(bridge, { type: "ack", id: frame.id });
    reportState(true);
    delivering = false;
    drainDeferredReceipts();
  }

  function handleBridgeFrame(state, frame) {
    if (!isCurrentBridge(state) || !frame || typeof frame.type !== "string") return;
    if (frame.type === "ready") {
      state.ready = true;
      bridgeReady = true;
      failedStarts = 0;
      backoff = 300;
      lastState = null;
      startPoll();
      reportState(true);
      return;
    }
    if (frame.type === "refused") {
      const reason = String(frame.reason ?? "");
      if (reason === "protocol_mismatch") {
        state.final = true;
        bridgeReady = false;
        stopPoll();
        rejectPendingCalls();
        notifyOnce(
          "protocol_mismatch",
          "A2AMX and this OMP extension disagree on the protocol version; update A2AMX.",
        );
      } else if (reason.startsWith("missing_apis: ")) {
        state.final = true;
        bridgeReady = false;
        stopPoll();
        rejectPendingCalls();
        notifyOnce(
          "missing_apis",
          `A2AMX needs OMP features this OMP version lacks (${reason.slice("missing_apis: ".length)}); update A2AMX or OMP.`,
        );
      } else if (reason === "session is not an omp session") {
        state.final = true;
        bridgeReady = false;
        stopPoll();
        rejectPendingCalls();
        notifyOnce(
          "non_omp_session",
          "This session was not started with a2amx new --harness omp.",
        );
      } else if (reason === "a bridge is already attached to this session") {
        state.retryOnClose = true;
        bridgeReady = false;
        stopPoll();
        rejectPendingCalls();
      } else {
        state.final = true;
        bridgeReady = false;
        stopPoll();
        rejectPendingCalls();
        notifyOnce(`refused:${reason}`, reason);
      }
      return;
    }
    if (frame.type === "deliver") {
      handleDeliver(frame);
      return;
    }
    if (frame.type === "mcp") {
      handleMcpResponse(frame.response);
    }
  }

  function parseStdout(state, chunk) {
    state.stdoutBuffer += String(chunk);
    let newline;
    while ((newline = state.stdoutBuffer.indexOf("\n")) >= 0) {
      let line = state.stdoutBuffer.slice(0, newline);
      state.stdoutBuffer = state.stdoutBuffer.slice(newline + 1);
      if (line.endsWith("\r")) line = line.slice(0, -1);
      if (line.length === 0) continue;
      let frame;
      try {
        frame = JSON.parse(line);
      } catch (error) {
        recordBridgeError(state, error);
        return;
      }
      handleBridgeFrame(state, frame);
    }
  }

  function parseStderr(state, chunk) {
    state.stderrTail = Buffer.concat([state.stderrTail, Buffer.from(chunk)]).subarray(-2048);
  }

  function failedStartNotice() {
    failedStarts += 1;
    if (failedStarts < 10) return;
    const line = lastStderrLine;
    notifyOnce(
      "cannot_reach_daemon",
      `A2AMX cannot reach its daemon (${line}); still retrying.`,
    );
  }

  function scheduleRespawn() {
    if (shuttingDown || respawnTimer !== null || bridge !== null) return;
    const delay = backoff;
    backoff = Math.min(backoff * 2, 5000);
    respawnTimer = setTimeout(() => {
      respawnTimer = null;
      spawnBridge();
    }, delay);
  }

  function recordBridgeError(state, error) {
    if (!isCurrentBridge(state) || state.processError) return;
    state.processError = error;
    if (state.stderrTail.length === 0) {
      lastStderrLine = String(error?.message ?? error).slice(-2048);
    }
    bridgeReady = false;
    late.length = 0;
    stopPoll();
    rejectPendingCalls();
    try {
      state.child.kill();
    } catch (error) {
      lastStderrLine = String(error?.message ?? error).slice(-2048);
    }
  }

  function finishBridge(state, code, signal) {
    if (state.finished) return;
    state.finished = true;
    if (state.stderrTail.length > 0) {
      lastStderrLine = state.stderrTail.toString("utf8").replace(/[\r\n]+$/, "").split("\n").pop();
    }
    if (bridge !== state) return;
    bridge = null;
    bridgeReady = false;
    lastState = null;
    late.length = 0;
    stopPoll();
    rejectPendingCalls();
    if (shuttingDown || state.final) return;

    const shouldRespawn = state.retryOnClose || code !== 0 || signal !== null;
    if (!shouldRespawn) return;
    if (!state.ready) failedStartNotice();
    scheduleRespawn();
  }

  function spawnBridge() {
    if (shuttingDown || bridge !== null || respawnTimer !== null) return;
    const env = process.env || {};
    const binary = env.A2AMX_BIN;
    let child;
    try {
      child = spawn(binary, ["omp-bridge"], {
        stdio: ["pipe", "pipe", "pipe"],
        env,
      });
    } catch (error) {
      lastStderrLine = String(error?.message ?? error).slice(-2048);
      failedStartNotice();
      scheduleRespawn();
      return;
    }

    const state = {
      child,
      finished: false,
      ready: false,
      final: false,
      retryOnClose: false,
      stdoutBuffer: "",
      stderrTail: Buffer.alloc(0),
    };
    bridge = state;
    bridgeReady = false;
    lastState = null;

    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => parseStdout(state, chunk));
    child.stderr.on("data", (chunk) => parseStderr(state, chunk));
    for (const stream of [child.stdout, child.stderr, child.stdin]) {
      stream.on("error", (error) => recordBridgeError(state, error));
    }
    child.on("error", (error) => recordBridgeError(state, error));
    child.on("close", (code, signal) => finishBridge(state, code, signal));

    sendFrame(state, {
      type: "hello",
      protocol: PROTOCOL,
      omp_version: helloVersion,
      missing: helloMissing,
    });
  }

  let helloMissing = [];
  let helloVersion = "unknown";

  function registerTools() {
    if (typeof pi?.registerTool !== "function") return;
    for (const tool of TOOLS) {
      try {
        pi.registerTool({
          name: tool.name,
          label: tool.name,
          description: tool.description,
          parameters: tool.inputSchema,
          execute: (toolCallId, params) => executeTool(tool.name, params),
        });
      } catch (error) {
        notifyOnce(
          `register_tool:${tool.name}`,
          String(error?.message ?? error),
        );
      }
    }
  }

  function executeTool(name, params) {
    if (!bridgeReady || !isCurrentBridge(bridge)) {
      throw new Error("A2AMX is not connected");
    }
    const argumentsObject = { ...(params || {}) };
    delete argumentsObject.i;
    const id = nextRequestId;
    nextRequestId += 1;
    const { promise, resolve, reject } = Promise.withResolvers();
    const timer = setTimeout(() => {
      pendingCalls.delete(id);
      reject(new Error("A2AMX did not answer in time"));
    }, 60000);
    pendingCalls.set(id, { resolve, reject, timer });
    if (
      !sendFrame(bridge, {
        type: "mcp",
        request: {
          jsonrpc: "2.0",
          id,
          method: "tools/call",
          params: { name, arguments: argumentsObject },
        },
      })
    ) {
      pendingCalls.delete(id);
      clearTimeout(timer);
      reject(new Error("A2AMX connection lost"));
    }
    return promise;
  }

  function shutdown() {
    if (!isMainSession()) return;
    shuttingDown = true;
    late.length = 0;
    if (respawnTimer !== null) {
      clearTimeout(respawnTimer);
      respawnTimer = null;
    }
    stopPoll();
    rejectPendingCalls();
    const state = bridge;
    bridgeReady = false;
    if (!state || state.finished) return;
    try {
      state.child.stdin.end();
    } catch (error) {
      lastStderrLine = String(error?.message ?? error).slice(-2048);
    }
    setTimeout(() => {
      if (!state.finished) {
        try {
          state.child.kill();
        } catch (error) {
          lastStderrLine = String(error?.message ?? error).slice(-2048);
        }
      }
    }, 1000);
  }

  pi.on("session_start", (_event, sessionContext) => {
    if (sessionContext?.agent?.kind !== "main") return;
    ctx = sessionContext;
    if (initialized) return;
    initialized = true;

    const environment = process.env || {};
    for (const name of ["A2AMX_BIN", "A2AMX_ADDR", "A2AMX_TOKEN"]) {
      if (typeof environment[name] !== "string" || environment[name].length === 0) {
        notifyOnce(
          `environment:${name}`,
          `A2AMX could not start: ${name} is not set. Start this session with a2amx new --harness omp.`,
        );
        return;
      }
    }

    helloMissing = missingApis(sessionContext);
    helloVersion = String(pi?.pi?.VERSION ?? "unknown");
    registerTools();
    spawnBridge();
  });

  for (const eventName of ["agent_start", "agent_end", "turn_start", "turn_end"]) {
    pi.on(eventName, () => {
      if (isMainSession()) reportState(false);
    });
  }
  pi.on("message_end", handleMessageEnd);
  pi.on("session_shutdown", shutdown);
}
