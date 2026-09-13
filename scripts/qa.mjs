#!/usr/bin/env node
// The QA build: a branch running as an app of its own beside the installed
// Vesper, driven through the WebView's DevTools protocol — never through the
// mouse, keyboard, focus or screen of whoever is using the machine.
//
// When and how to use it: AGENTS.md, "Ambiente de QA".

import { execFileSync, spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// Short on purpose — the Vulkan build aborts past ~260 characters (AGENTS.md) —
// and shared by every worktree, which is what spares each branch from compiling
// llama.cpp and whisper.cpp again.
const targetDir = process.env.CARGO_TARGET_DIR || "C:\\vt";
const exe = path.join(targetDir, "release", "vesper.exe");
const port = 9333;
const appData = process.env.APPDATA ?? "";
const installedData = path.join(appData, "Vesper");
// Both follow from `tauri.qa.conf.json`: the folder `Profile::Qa` names, and the
// one WebView2 keeps under the identifier.
const qaData = path.join(appData, "Vesper QA");
const qaWebView = path.join(process.env.LOCALAPPDATA ?? "", "com.yefclub.vesper.qa");

// Set on this one process, never on the machine: in the user's environment it
// would open a debugging port in every WebView2 app they start.
const browserArgs = [
  `--remote-debugging-port=${port}`,
  // Keep painting behind other windows, so nothing has to come forward to be
  // tested.
  "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding",
  "--disable-background-timer-throttling",
  // wry's own defaults repeated, in case this variable replaces what the app
  // passes rather than adding to it (WebView2Feedback#5571).
  "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion",
].join(" ");

const usage = `npm run qa -- <command>

  build                release build with gpu-vulkan, the way the installed app is built
  start                run it in the background, DevTools on 127.0.0.1:${port}
  stop                 end it
  reset                end it and delete the QA copy's own data
  cdp targets
  cdp eval <js>
  cdp click <data-testid>
  cdp type <text>
  cdp key <combo>      Tab, Shift+Tab, Enter, Escape, ArrowDown, Space, a
  cdp shot <file.png>
                       --overlay addresses the overlay window instead of the main one`;

function fail(message) {
  console.error(message);
  process.exit(1);
}

function readJson(relative) {
  return JSON.parse(fs.readFileSync(path.join(root, relative), "utf8"));
}

function build() {
  stop();
  const base = readJson("src-tauri/tauri.conf.json");
  const qa = readJson("src-tauri/tauri.qa.conf.json");
  // Only what the overlay changes goes to `--config`, so no relative path is
  // left for the CLI to resolve against the patch's location. The window list
  // is carried whole because a patch replaces arrays, and only the first window
  // takes the QA fields — the merge `dev-channel.yml` applies.
  const patch = structuredClone(qa);
  patch.app.windows = base.app.windows.map((window, i) =>
    i === 0 ? { ...window, ...qa.app.windows[0] } : window,
  );
  fs.mkdirSync(targetDir, { recursive: true });
  const patchFile = path.join(targetDir, "tauri.qa.json");
  fs.writeFileSync(patchFile, JSON.stringify(patch, null, 2));

  const env = { ...process.env, CARGO_TARGET_DIR: targetDir, CMAKE_GENERATOR: "Ninja" };
  // Windows spells it `Path`; a second `PATH` beside it leaves which one the
  // child sees to chance.
  const pathKey = Object.keys(env).find((key) => key.toUpperCase() === "PATH") ?? "PATH";
  env[pathKey] = [...buildTools(), env[pathKey]].join(path.delimiter);

  const cli = path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
  const { status } = spawnSync(
    process.execPath,
    [cli, "build", "--no-bundle", "--features", "gpu-vulkan", "--config", patchFile],
    { cwd: root, env, stdio: "inherit" },
  );
  if (status !== 0) fail(`build failed with exit code ${status}`);
  console.log(`built ${exe}`);
}

// The two tools AGENTS.md says a Windows build cannot do without and does not
// find by itself: LLVM's nm/objcopy, and the Ninja generator Vulkan needs —
// which the Visual Studio Build Tools already carry.
function buildTools() {
  const dirs = [];
  const llvm = "C:\\Program Files\\LLVM\\bin";
  if (fs.existsSync(llvm)) dirs.push(llvm);
  if (spawnSync("where", ["ninja"], { stdio: "ignore" }).status !== 0) {
    const vswhere = path.join(
      process.env["ProgramFiles(x86)"] ?? "",
      "Microsoft Visual Studio",
      "Installer",
      "vswhere.exe",
    );
    const vs = fs.existsSync(vswhere)
      ? execFileSync(vswhere, ["-latest", "-products", "*", "-property", "installationPath"], {
          encoding: "utf8",
        }).trim()
      : "";
    const ninja = path.join(vs, "Common7", "IDE", "CommonExtensions", "Microsoft", "CMake", "Ninja");
    if (!vs || !fs.existsSync(path.join(ninja, "ninja.exe"))) {
      fail("Ninja not found: install it, or the CMake component of the Visual Studio Build Tools");
    }
    dirs.push(ninja);
  }
  return dirs;
}

// Filtered by full path inside WMI, so no other process is even listed. The
// installed app is a vesper.exe too, and it is never this script's to stop.
function qaPids() {
  const wql = exe.replaceAll("\\", "\\\\").replaceAll("'", "\\'");
  const script = `Get-CimInstance Win32_Process -Filter "ExecutablePath = '${wql}'" | ForEach-Object { $_.ProcessId }`;
  return execFileSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8" })
    .split(/\s+/)
    .filter(Boolean);
}

function stop() {
  const pids = qaPids();
  for (const pid of pids) {
    spawnSync("taskkill", ["/PID", pid, "/T", "/F"], { stdio: "ignore" });
    console.log(`stopped ${pid}`);
  }
  // `taskkill` returns before the process is gone, and until it is, the
  // database is still open: a reset straight after a stop failed on it.
  const pause = new Int32Array(new SharedArrayBuffer(4));
  for (let attempt = 0; pids.length && attempt < 30 && qaPids().length; attempt++) {
    Atomics.wait(pause, 0, 0, 500);
  }
}

// Models and downloaded backends are copied from the installed app on the first
// start, so nothing is fetched again. Copied, not hard-linked: a link shares the
// file, and the backend unpacker and the digest markers rewrite files in place —
// through a link, into the installed app's copy. Unfinished downloads are left
// to the app that started them.
function copyMissing(from, to) {
  if (!fs.existsSync(from)) return;
  fs.mkdirSync(to, { recursive: true });
  for (const entry of fs.readdirSync(from, { withFileTypes: true })) {
    const source = path.join(from, entry.name);
    const target = path.join(to, entry.name);
    if (entry.isDirectory()) {
      copyMissing(source, target);
      continue;
    }
    // A target that still shares its file — a link left by an earlier version
    // of this script, or made by hand — is replaced: the copy is the isolation.
    if (fs.existsSync(target) && fs.statSync(target).nlink > 1) fs.unlinkSync(target);
    if (entry.isFile() && !/\.(part|etag)$/.test(entry.name) && !fs.existsSync(target)) {
      // Renamed into place, so an interrupted copy is never taken for a model.
      fs.copyFileSync(source, `${target}.qa-copy`);
      fs.renameSync(`${target}.qa-copy`, target);
    }
  }
}

async function pages() {
  const response = await fetch(`http://127.0.0.1:${port}/json/list`);
  return (await response.json()).filter((target) => target.type === "page");
}

// Any listener at all, not only a DevTools one: WebView2 cannot take a port that
// is held, and asking for DevTools pages of something that answers otherwise
// used to read as a free port.
function portTaken() {
  return new Promise((resolve) => {
    const socket = net.connect({ host: "127.0.0.1", port }, () => {
      socket.destroy();
      resolve(true);
    });
    socket.on("error", () => resolve(false));
  });
}

async function start() {
  if (!fs.existsSync(exe)) fail(`no QA build at ${exe}: npm run qa -- build`);
  if (qaPids().length) fail("the QA build is already running: npm run qa -- stop");
  if (await portTaken()) fail(`port ${port} is already in use: find out what holds it`);
  for (const dir of ["models", "backends"]) {
    copyMissing(path.join(installedData, dir), path.join(qaData, dir));
  }

  const child = spawn(exe, [], {
    cwd: path.dirname(exe),
    detached: true,
    stdio: "ignore",
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: browserArgs },
  });
  child.unref();
  for (let attempt = 0; attempt < 120; attempt++) {
    const found = await pages().catch(() => []);
    // Not the first page listed: that can be the `about:blank` WebView2 opens
    // before the app loads, and a `cdp` command straight after would find no app.
    if (found.some((page) => page.url.startsWith("http://tauri.localhost/"))) {
      console.log(`running as ${child.pid}; pages:`);
      for (const page of found) console.log(`  ${page.url}`);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  // Nothing can drive a copy DevTools does not reach, so it is not left running.
  stop();
  fail("started, but DevTools never answered; stopped it again");
}

function reset() {
  stop();
  // Only ever these two: the QA copy's data and its WebView profile.
  for (const dir of [qaData, qaWebView]) {
    try {
      fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });
    } catch (e) {
      fail(`${dir} is still in use (${e.code}); run reset again in a few seconds`);
    }
  }
  console.log("QA data removed");
}

function connect(url) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url);
    const waiting = new Map();
    let last = 0;
    socket.onerror = () => reject(new Error(`cannot open ${url}`));
    socket.onmessage = ({ data }) => {
      const message = JSON.parse(data);
      const waiter = waiting.get(message.id);
      if (!waiter) return;
      waiting.delete(message.id);
      if (message.error) waiter.reject(new Error(message.error.message));
      else waiter.resolve(message.result);
    };
    socket.onopen = () =>
      resolve({
        send(method, params = {}) {
          return new Promise((done, broke) => {
            const id = ++last;
            // A window nothing paints never answers a screenshot. Say so
            // instead of hanging.
            const timer = setTimeout(() => {
              waiting.delete(id);
              broke(new Error(`${method} had no answer after 15 s`));
            }, 15_000);
            waiting.set(id, {
              resolve: (value) => {
                clearTimeout(timer);
                done(value);
              },
              reject: (error) => {
                clearTimeout(timer);
                broke(error);
              },
            });
            socket.send(JSON.stringify({ id, method, params }));
          });
        },
        close: () => socket.close(),
      });
  });
}

async function evaluate(session, expression) {
  const { result, exceptionDetails } = await session.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  // A Tauri command rejects with a plain string, which has a value but no
  // description; without it every backend refusal reads "Uncaught (in promise)".
  if (exceptionDetails) {
    const { exception, text } = exceptionDetails;
    fail(exception?.description ?? exception?.value ?? text);
  }
  return result.value;
}

const KEY_CODES = {
  Backspace: 8,
  Tab: 9,
  Enter: 13,
  Escape: 27,
  " ": 32,
  PageUp: 33,
  PageDown: 34,
  End: 35,
  Home: 36,
  ArrowLeft: 37,
  ArrowUp: 38,
  ArrowRight: 39,
  ArrowDown: 40,
  Delete: 46,
};
const MODIFIER_BITS = { Alt: 1, Control: 2, Meta: 4, Shift: 8 };

const cdpCommands = {
  async eval(session, [expression = ""]) {
    console.log(JSON.stringify(await evaluate(session, expression), null, 2));
  },

  // Pressed at the element's centre, inside the page — and only when the
  // element is what sits there. A click that would land on something covering
  // it is reported, not performed.
  async click(session, [testid = ""]) {
    const at = await evaluate(
      session,
      `(() => {
        const el = document.querySelector('[data-testid="' + CSS.escape(${JSON.stringify(testid)}) + '"]');
        if (!el) return { miss: true };
        el.scrollIntoView({ block: "center", inline: "center" });
        const r = el.getBoundingClientRect();
        const x = r.left + r.width / 2;
        const y = r.top + r.height / 2;
        const top = document.elementFromPoint(x, y);
        return el.contains(top) ? { x, y } : { covered: top ? top.outerHTML.slice(0, 160) : "nothing" };
      })()`,
    );
    if (at.miss) fail(`MISS: ${testid}`);
    if (at.covered) fail(`COVERED: ${testid} is under ${at.covered}`);
    for (const type of ["mousePressed", "mouseReleased"]) {
      await session.send("Input.dispatchMouseEvent", {
        type,
        x: at.x,
        y: at.y,
        button: "left",
        clickCount: 1,
      });
    }
    console.log(`clicked: ${testid}`);
  },

  async type(session, [text = ""]) {
    await session.send("Input.insertText", { text });
    console.log(`typed ${text.length} characters`);
  },

  async key(session, [combo = ""]) {
    const parts = combo.split("+");
    const named = parts.pop();
    const key = named === "Space" ? " " : named;
    let modifiers = 0;
    for (const part of parts) {
      if (!Object.hasOwn(MODIFIER_BITS, part)) fail(`unknown modifier: ${part}`);
      modifiers |= MODIFIER_BITS[part];
    }
    const single = key.length === 1;
    if (!single && !Object.hasOwn(KEY_CODES, key)) fail(`unknown key: ${key}`);
    const vk = Object.hasOwn(KEY_CODES, key) ? KEY_CODES[key] : key.toUpperCase().charCodeAt(0);
    const code = key === " " ? "Space" : !single ? key : /\d/.test(key) ? `Digit${key}` : `Key${key.toUpperCase()}`;
    const text = key === "Enter" ? "\r" : single && (modifiers & ~MODIFIER_BITS.Shift) === 0 ? key : undefined;
    const event = { key, code, windowsVirtualKeyCode: vk, modifiers };
    await session.send("Input.dispatchKeyEvent", { ...event, type: text ? "keyDown" : "rawKeyDown", text });
    await session.send("Input.dispatchKeyEvent", { ...event, type: "keyUp" });
    console.log(`pressed: ${combo}`);
  },

  async shot(session, [file = "qa.png"]) {
    const { data } = await session.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(file, Buffer.from(data, "base64"));
    console.log(path.resolve(file));
  },
};

async function cdp(args) {
  const overlay = args.includes("--overlay");
  const [command, ...rest] = args.filter((arg) => arg !== "--overlay");
  const found = await pages().catch(() => fail("the QA build is not running: npm run qa -- start"));
  if (command === "targets") {
    for (const page of found) console.log(page.url);
    return;
  }
  if (!Object.hasOwn(cdpCommands, command ?? "")) fail(usage);
  // The app's own pages only: WebView2 also lists an `about:blank` while a
  // window is being set up, and it can come first.
  const page = found.find(
    (p) => p.url.startsWith("http://tauri.localhost/") && p.url.includes("overlay.html") === overlay,
  );
  if (!page) fail(`no ${overlay ? "overlay" : "main"} page is open`);
  const session = await connect(page.webSocketDebuggerUrl);
  try {
    await cdpCommands[command](session, rest);
  } finally {
    session.close();
  }
}

if (process.platform !== "win32") {
  fail("The QA build is Windows-only: it drives WebView2 over the DevTools protocol.");
}
const [command, ...args] = process.argv.slice(2);
switch (command) {
  case "build":
    build();
    break;
  case "start":
    await start();
    break;
  case "stop":
    stop();
    break;
  case "reset":
    reset();
    break;
  case "cdp":
    await cdp(args);
    break;
  default:
    fail(usage);
}
