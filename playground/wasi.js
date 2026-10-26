// The little of WASI (preview 1) the playground's engine needs, for browsers and Node alike:
// arguments, environment, standard output and error, clocks and randomness. There are no files;
// anything else answers ENOSYS.

const SUCCESS = 0;
const EBADF = 8;
const ENOENT = 44;
const ENOSYS = 52;
const ESPIPE = 70;

/** What `proc_exit` throws to stop the program. */
export class Exit extends Error {
  constructor(code) {
    super(`exit ${code}`);
    this.code = code;
  }
}

/**
 * Runs a compiled WASI command module to completion and returns its exit status. `stdout` and
 * `stderr` receive the bytes the program writes (Uint8Arrays).
 */
export async function runWasi(module, options) {
  let memory;
  const imports = wasiImports(options, () => memory);
  const instance = await WebAssembly.instantiate(module, { wasi_snapshot_preview1: imports });
  memory = instance.exports.memory;
  try {
    instance.exports._start();
    return 0;
  } catch (e) {
    if (e instanceof Exit) return e.code;
    throw e;
  }
}

/** The WASI calls, for an instance whose memory `memoryOf()` returns once it exists. */
export function wasiImports({ args, env = {}, stdout, stderr }, memoryOf) {
  const encoder = new TextEncoder();
  const argv = args.map((a) => encoder.encode(`${a}\0`));
  const envs = Object.entries(env).map(([k, v]) => encoder.encode(`${k}=${v}\0`));
  const memory = {
    get buffer() {
      return memoryOf().buffer;
    },
  };
  const view = () => new DataView(memory.buffer);
  const bytes = () => new Uint8Array(memory.buffer);

  const putStrings = (list, pointers, buffer) => {
    let at = buffer;
    list.forEach((s, i) => {
      view().setUint32(pointers + 4 * i, at, true);
      bytes().set(s, at);
      at += s.length;
    });
    return SUCCESS;
  };
  const putSizes = (list, count, size) => {
    view().setUint32(count, list.length, true);
    view().setUint32(size, list.reduce((n, s) => n + s.length, 0), true);
    return SUCCESS;
  };

  const calls = {
    args_get: (pointers, buffer) => putStrings(argv, pointers, buffer),
    args_sizes_get: (count, size) => putSizes(argv, count, size),
    environ_get: (pointers, buffer) => putStrings(envs, pointers, buffer),
    environ_sizes_get: (count, size) => putSizes(envs, count, size),
    fd_write(fd, iovs, count, written) {
      if (fd !== 1 && fd !== 2) return EBADF;
      let total = 0;
      for (let i = 0; i < count; i++) {
        const v = view();
        const ptr = v.getUint32(iovs + 8 * i, true);
        const len = v.getUint32(iovs + 8 * i + 4, true);
        (fd === 1 ? stdout : stderr)(bytes().slice(ptr, ptr + len));
        total += len;
      }
      view().setUint32(written, total, true);
      return SUCCESS;
    },
    // Standard input is empty.
    fd_read(fd, iovs, count, read) {
      if (fd !== 0) return EBADF;
      view().setUint32(read, 0, true);
      return SUCCESS;
    },
    fd_fdstat_get(fd, out) {
      if (fd > 2) return EBADF;
      const v = view();
      for (let i = 0; i < 24; i++) v.setUint8(out + i, 0);
      v.setUint8(out, 2); // a character device
      v.setBigUint64(out + 8, 0xffffffffffffffffn, true);
      v.setBigUint64(out + 16, 0xffffffffffffffffn, true);
      return SUCCESS;
    },
    fd_close: (fd) => (fd <= 2 ? SUCCESS : EBADF),
    fd_seek: (fd) => (fd <= 2 ? ESPIPE : EBADF),
    fd_prestat_get: () => EBADF, // no directories are open, so there are no files
    fd_prestat_dir_name: () => EBADF,
    path_open: () => ENOENT,
    proc_exit(code) {
      throw new Exit(code);
    },
    clock_res_get(id, out) {
      view().setBigUint64(out, 1000n, true);
      return SUCCESS;
    },
    clock_time_get(id, precision, out) {
      const ms = id === 0 ? Date.now() : performance.now();
      view().setBigUint64(out, BigInt(Math.round(ms * 1e6)), true);
      return SUCCESS;
    },
    random_get(buffer, length) {
      for (let at = 0; at < length; at += 65536) {
        crypto.getRandomValues(bytes().subarray(buffer + at, buffer + Math.min(length, at + 65536)));
      }
      return SUCCESS;
    },
    sched_yield: () => SUCCESS,
  };
  return new Proxy(calls, { get: (target, name) => target[name] ?? (() => ENOSYS) });
}
