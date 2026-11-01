// The NEC engine in a browser Web Worker: the browser's version of the
// desktop app's worker processes. Each message runs one deck through nec2c
// (straws-engine.wasm, built for wasm32-wasip1) in a fresh WebAssembly
// instance, so nec2c's global state and exit() calls touch nothing else,
// and answers with the output text and the binary pattern/current file.
//
// The engine sees a tiny in-memory file system through the WASI calls
// below: only what nec2c and Rust's standard library actually use.
//
// Message in:  { id, deck, binary, add, drop }
//   binary: write patterns and currents to a binary file, as on the desktop;
//   add: [[name, ArrayBuffer]] Sommerfeld ground tables to keep (worked out
//   by other workers); drop: names of tables to forget.
// Message out: { id, code, output, stderr, binary (ArrayBuffer or null),
//                fresh: [[name, ArrayBuffer]] (tables this run worked out),
//                ms } or { id, error }
//
// The ground tables are what the desktop keeps in its cache folder (see
// somnec.c): here each worker keeps them in memory between runs, and the
// app passes new ones on to the other workers.

"use strict";

let modulePromise = null;

function engineModule() {
  if (!modulePromise) {
    const url = new URL("straws-engine.wasm", self.location.href);
    modulePromise = WebAssembly.compileStreaming(fetch(url)).catch(async () => {
      // Some servers send the wrong content type; fall back to bytes.
      const bytes = await (await fetch(url)).arrayBuffer();
      return WebAssembly.compile(bytes);
    });
  }
  return modulePromise;
}

class Exit {
  constructor(code) {
    this.code = code;
  }
}

const ERRNO = { SUCCESS: 0, BADF: 8, INVAL: 28, ISDIR: 31, NOENT: 44, NOSYS: 52 };
const FILETYPE = { CHARACTER_DEVICE: 2, DIRECTORY: 3, REGULAR_FILE: 4 };
const OFLAGS = { CREAT: 1, DIRECTORY: 2, EXCL: 4, TRUNC: 8 };
const FDFLAGS = { APPEND: 1 };
const PREOPEN = 3;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

// A growable byte buffer standing in for a file.
class MemFile {
  constructor(bytes) {
    this.data = bytes || new Uint8Array(0);
    this.size = this.data.length;
  }
  bytes() {
    return this.data.subarray(0, this.size);
  }
  write(pos, chunk) {
    const end = pos + chunk.length;
    if (end > this.data.length) {
      const grown = new Uint8Array(Math.max(end, this.data.length * 2, 4096));
      grown.set(this.data.subarray(0, this.size));
      this.data = grown;
    }
    this.data.set(chunk, pos);
    this.size = Math.max(this.size, end);
  }
}

// Runs nec2c once: files maps names to MemFiles (in and out).
function runEngine(module, args, env, files) {
  const fds = new Map(); // fd -> { file, pos, append }
  let nextFd = PREOPEN + 1;
  const stdout = [];
  const stderr = [];
  let memory = null;

  const view = () => new DataView(memory.buffer);
  const bytes = () => new Uint8Array(memory.buffer);
  const text = (ptr, len) => decoder.decode(bytes().subarray(ptr, ptr + len));
  const cleanPath = (p) => p.replace(/^(\.\/|\/)+/, "");

  // Writes a list of strings as C strings plus a pointer table.
  function putStrings(list, tablePtr, bufPtr) {
    const v = view();
    const b = bytes();
    let at = bufPtr;
    list.forEach((s, k) => {
      v.setUint32(tablePtr + 4 * k, at, true);
      const enc = encoder.encode(s);
      b.set(enc, at);
      b[at + enc.length] = 0;
      at += enc.length + 1;
    });
    return ERRNO.SUCCESS;
  }
  function sizes(list, countPtr, sizePtr) {
    const v = view();
    v.setUint32(countPtr, list.length, true);
    v.setUint32(sizePtr, list.reduce((n, s) => n + encoder.encode(s).length + 1, 0), true);
    return ERRNO.SUCCESS;
  }
  function iovecs(ptr, count) {
    const v = view();
    const out = [];
    for (let k = 0; k < count; k++) {
      out.push([v.getUint32(ptr + 8 * k, true), v.getUint32(ptr + 8 * k + 4, true)]);
    }
    return out;
  }

  const wasi = {
    args_sizes_get: (c, s) => sizes(args, c, s),
    args_get: (t, b) => putStrings(args, t, b),
    environ_sizes_get: (c, s) => sizes(env, c, s),
    environ_get: (t, b) => putStrings(env, t, b),
    clock_time_get: (_id, _precision, ptr) => {
      const ns = BigInt(Math.round((performance.timeOrigin + performance.now()) * 1e6));
      view().setBigUint64(ptr, ns, true);
      return ERRNO.SUCCESS;
    },
    fd_close: (fd) => (fds.delete(fd) ? ERRNO.SUCCESS : ERRNO.BADF),
    fd_fdstat_get: (fd, ptr) => {
      let type;
      if (fd <= 2) type = FILETYPE.CHARACTER_DEVICE;
      else if (fd === PREOPEN) type = FILETYPE.DIRECTORY;
      else if (fds.has(fd)) type = FILETYPE.REGULAR_FILE;
      else return ERRNO.BADF;
      const v = view();
      v.setUint8(ptr, type);
      v.setUint16(ptr + 2, fds.get(fd)?.append ? FDFLAGS.APPEND : 0, true);
      v.setBigUint64(ptr + 8, 0xffffffffffffffffn, true);
      v.setBigUint64(ptr + 16, 0xffffffffffffffffn, true);
      return ERRNO.SUCCESS;
    },
    fd_fdstat_set_flags: () => ERRNO.SUCCESS,
    fd_prestat_get: (fd, ptr) => {
      if (fd !== PREOPEN) return ERRNO.BADF;
      const v = view();
      v.setUint8(ptr, 0);
      v.setUint32(ptr + 4, 1, true);
      return ERRNO.SUCCESS;
    },
    fd_prestat_dir_name: (fd, ptr, len) => {
      if (fd !== PREOPEN) return ERRNO.BADF;
      if (len >= 1) bytes()[ptr] = 0x2f; // "/"
      return ERRNO.SUCCESS;
    },
    fd_read: (fd, iovs, count, nreadPtr) => {
      const open = fds.get(fd);
      let total = 0;
      if (open) {
        const data = open.file.bytes();
        for (const [buf, len] of iovecs(iovs, count)) {
          const chunk = data.subarray(open.pos, Math.min(open.pos + len, data.length));
          bytes().set(chunk, buf);
          open.pos += chunk.length;
          total += chunk.length;
          if (chunk.length < len) break;
        }
      } else if (fd !== 0) {
        return ERRNO.BADF;
      }
      view().setUint32(nreadPtr, total, true);
      return ERRNO.SUCCESS;
    },
    fd_seek: (fd, offset, whence, newPtr) => {
      const open = fds.get(fd);
      if (!open) return ERRNO.BADF;
      const base = whence === 0 ? 0 : whence === 1 ? open.pos : open.file.size;
      const pos = base + Number(offset);
      if (pos < 0) return ERRNO.INVAL;
      open.pos = pos;
      view().setBigUint64(newPtr, BigInt(pos), true);
      return ERRNO.SUCCESS;
    },
    fd_write: (fd, iovs, count, nwrittenPtr) => {
      let total = 0;
      for (const [buf, len] of iovecs(iovs, count)) {
        const chunk = bytes().slice(buf, buf + len);
        if (fd === 1) stdout.push(chunk);
        else if (fd === 2) stderr.push(chunk);
        else {
          const open = fds.get(fd);
          if (!open) return ERRNO.BADF;
          if (open.append) open.pos = open.file.size;
          open.file.write(open.pos, chunk);
          open.pos += chunk.length;
        }
        total += len;
      }
      view().setUint32(nwrittenPtr, total, true);
      return ERRNO.SUCCESS;
    },
    path_open: (dirFd, _dirFlags, pathPtr, pathLen, oflags, _rb, _ri, fdflags, fdPtr) => {
      if (dirFd !== PREOPEN) return ERRNO.BADF;
      const name = cleanPath(text(pathPtr, pathLen));
      if (name === "" || oflags & OFLAGS.DIRECTORY) return ERRNO.ISDIR;
      let file = files.get(name);
      if (!file) {
        if (!(oflags & OFLAGS.CREAT)) return ERRNO.NOENT;
        file = new MemFile();
        files.set(name, file);
      } else if (oflags & OFLAGS.TRUNC) {
        file.size = 0;
      }
      const fd = nextFd++;
      fds.set(fd, { file, pos: 0, append: (fdflags & FDFLAGS.APPEND) !== 0 });
      view().setUint32(fdPtr, fd, true);
      return ERRNO.SUCCESS;
    },
    path_unlink_file: (dirFd, pathPtr, pathLen) =>
      files.delete(cleanPath(text(pathPtr, pathLen))) ? ERRNO.SUCCESS : ERRNO.NOENT,
    path_remove_directory: () => ERRNO.NOENT,
    path_rename: (fromFd, fromPtr, fromLen, toFd, toPtr, toLen) => {
      const from = cleanPath(text(fromPtr, fromLen));
      const file = files.get(from);
      if (!file) return ERRNO.NOENT;
      files.delete(from);
      files.set(cleanPath(text(toPtr, toLen)), file);
      return ERRNO.SUCCESS;
    },
    proc_exit: (code) => {
      throw new Exit(code);
    },
  };

  // Anything else nec2c might reach answers "not supported".
  const imports = {
    wasi_snapshot_preview1: new Proxy(wasi, {
      get: (target, name) => target[name] || (() => ERRNO.NOSYS),
    }),
  };

  const instance = new WebAssembly.Instance(module, imports);
  memory = instance.exports.memory;
  let code = 0;
  try {
    instance.exports._start();
  } catch (e) {
    if (e instanceof Exit) code = e.code;
    else throw e;
  }
  const join = (parts) => decoder.decode(concat(parts));
  return { code, stdout: join(stdout), stderr: join(stderr) };
}

function concat(parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

// Sommerfeld ground tables this worker holds, by file name.
const tables = new Map();
const TABLES = "som/";

self.onmessage = async (event) => {
  const { id, deck, binary, add, drop } = event.data;
  for (const name of drop || []) tables.delete(name);
  for (const [name, buffer] of add || []) tables.set(name, new Uint8Array(buffer));
  try {
    const module = await engineModule();
    const started = performance.now();
    const files = new Map([["deck.nec", new MemFile(encoder.encode(deck))]]);
    for (const [name, bytes] of tables) files.set(TABLES + name, new MemFile(bytes));
    const env = ["NEC_SOLVER_THREADS=1", "STRAWS_SOMNEC_CACHE=som"];
    if (binary) env.push("STRAWS_BINARY_PATTERN=deck.pat");
    const run = runEngine(
      module,
      ["straws-worker", "-i", "deck.nec", "-o", "deck.out"],
      env,
      files,
    );
    const output = files.has("deck.out") ? decoder.decode(files.get("deck.out").bytes()) : "";
    const pat = files.get("deck.pat");
    const bin = pat ? pat.bytes().slice().buffer : null;
    const fresh = [];
    for (const [path, file] of files) {
      if (!path.startsWith(TABLES) || !path.endsWith(".bin")) continue;
      const name = path.slice(TABLES.length);
      if (tables.has(name)) continue;
      const bytes = file.bytes().slice();
      tables.set(name, bytes);
      fresh.push([name, bytes.slice().buffer]);
    }
    const transfer = fresh.map(([, b]) => b);
    if (bin) transfer.push(bin);
    self.postMessage(
      {
        id,
        code: run.code,
        output,
        stderr: run.stderr + run.stdout,
        binary: bin,
        fresh,
        ms: performance.now() - started,
      },
      transfer,
    );
  } catch (e) {
    // A trap (a crash inside the engine) ends only this run.
    self.postMessage({ id, error: String(e && e.message ? e.message : e) });
  }
};
