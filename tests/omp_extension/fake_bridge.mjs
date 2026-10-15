#!/usr/bin/env node
import fs from "node:fs";
import net from "node:net";

const pidDir = process.env.FAKE_BRIDGE_PID_DIR;
if (!pidDir) process.exit(1);
fs.mkdirSync(pidDir, { recursive: true });
const marker = `${pidDir}/${process.pid}`;
fs.writeFileSync(marker, "");
const port = Number(process.env.FAKE_BRIDGE_PORT);
const socket = net.connect({ host: "127.0.0.1", port });
let ended = false;
const finish = (code) => {
  if (ended) return;
  ended = true;
  // Keep the marker until harness teardown so it can observe the process's ESRCH.
  process.exit(code);
};
socket.on("connect", () => {
  process.stdin.on("data", (chunk) => socket.write(chunk));
  process.stdin.on("end", () => {
    if (process.env.FAKE_BRIDGE_IGNORE_STDIN_END !== "1") {
      socket.end();
      finish(0);
    }
  });
});
socket.on("data", (chunk) => process.stdout.write(chunk));
socket.on("close", () => finish(1));
socket.on("error", () => finish(1));
process.stdin.on("error", () => finish(1));
