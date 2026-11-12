// Drives headless Chromium over the DevTools protocol to check the WebAssembly
// build: loads the page, optionally sends key presses and clicks, saves a
// screenshot and prints console errors.
//
//   node tools/web-check.mjs --url http://127.0.0.1:8080/ --out shot.png \
//        [--wait 12] [--keys "w:1500,Space:200"] [--size 1280x720]
//
// Requires Node 22+ (built-in WebSocket) and a `chromium` binary on PATH.

import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, arg, i, all) => (arg.startsWith("--") ? [...acc, [arg.slice(2), all[i + 1]]] : acc), []),
);
const url = args.url ?? "http://127.0.0.1:8080/";
const out = args.out ?? "web-check.png";
const waitSeconds = Number(args.wait ?? 12);
const [width, height] = (args.size ?? "1280x720").split("x").map(Number);
const port = 9333 + Math.floor(Math.random() * 500);
const profile = mkdtempSync(join(tmpdir(), "araria-web-check-"));

const chrome = spawn(
  process.env.CHROME ?? "chromium",
  [
    "--headless=new",
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    `--window-size=${width},${height}`,
    "--enable-unsafe-swiftshader",
    "--ignore-gpu-blocklist",
    "--use-angle=swiftshader",
    "--no-first-run",
    "--mute-audio",
    "about:blank",
  ],
  { stdio: "ignore" },
);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function connect() {
  for (let attempt = 0; attempt < 50; attempt++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const page = targets.find((t) => t.type === "page");
      if (page) return new WebSocket(page.webSocketDebuggerUrl);
    } catch {
      // Chromium is still starting.
    }
    await sleep(200);
  }
  throw new Error("could not connect to Chromium");
}

const KEY_CODES = { Space: [" ", "Space", 32], Shift: ["Shift", "ShiftLeft", 16], Enter: ["Enter", "Enter", 13], Escape: ["Escape", "Escape", 27] };

function keyEvent(type, name) {
  const [key, code, vk] = KEY_CODES[name] ?? [name.toLowerCase(), `Key${name.toUpperCase()}`, name.toUpperCase().charCodeAt(0)];
  return { type, key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk };
}

let failed = false;
try {
  const socket = await connect();
  await new Promise((resolve) => (socket.onopen = resolve));
  let nextId = 1;
  const pending = new Map();
  const logs = [];
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      pending.get(message.id)(message.result ?? message);
      pending.delete(message.id);
    } else if (message.method === "Runtime.consoleAPICalled") {
      const text = message.params.args.map((a) => a.value ?? a.description ?? "").join(" ");
      logs.push([message.params.type, text]);
    } else if (message.method === "Runtime.exceptionThrown") {
      const details = message.params.exceptionDetails;
      logs.push(["exception", details.exception?.description ?? details.text]);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve) => {
      const id = nextId++;
      pending.set(id, resolve);
      socket.send(JSON.stringify({ id, method, params }));
    });

  await send("Runtime.enable");
  await send("Page.enable");
  await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
  await send("Page.navigate", { url });
  await sleep(waitSeconds * 1000);

  // Give the canvas focus, then replay the requested input.
  for (const type of ["mousePressed", "mouseReleased"]) {
    await send("Input.dispatchMouseEvent", { type, x: width / 2, y: height / 2, button: "left", clickCount: 1 });
  }
  for (const step of (args.keys ?? "").split(",").filter(Boolean)) {
    const [name, duration] = step.split(":");
    await send("Input.dispatchKeyEvent", keyEvent("keyDown", name));
    await sleep(Number(duration ?? 100));
    await send("Input.dispatchKeyEvent", keyEvent("keyUp", name));
    await sleep(50);
  }
  await sleep(500);

  const shot = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(out, Buffer.from(shot.data, "base64"));
  console.log(`screenshot: ${out}`);

  const problems = logs.filter(([type, text]) => type === "error" || type === "exception" || /panicked|ERROR/.test(text));
  for (const [type, text] of problems.slice(0, 20)) console.log(`[${type}] ${text.slice(0, 400)}`);
  console.log(`console: ${logs.length} messages, ${problems.length} problems`);
  failed = problems.length > 0;
  socket.close();
} finally {
  chrome.kill();
  await sleep(300);
  rmSync(profile, { recursive: true, force: true });
}
process.exit(failed ? 1 : 0);
