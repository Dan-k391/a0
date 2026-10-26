#!/usr/bin/env python3
"""Measures `minnow lsp` on the generated large package (gen.py): how long the language server
takes to publish diagnostics when a file is opened and after an edit, and to answer hover,
go-to-definition and completion requests.

    python3 bench/large/lsp.py <minnow> <package dir> [--runs 5]

Prints a Markdown table of medians in milliseconds.
"""

import argparse
import json
import os
import statistics
import subprocess
import threading
import time
from pathlib import Path


class Client:
    def __init__(self, cmd, cwd):
        self.proc = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=None if os.environ.get("A0_TIME") else subprocess.DEVNULL)
        self.next_id = 0
        self.messages = []
        self.cond = threading.Condition()
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        out = self.proc.stdout
        while True:
            length = None
            while True:
                line = out.readline()
                if not line:
                    return
                line = line.strip()
                if not line:
                    break
                if line.lower().startswith(b"content-length:"):
                    length = int(line.split(b":")[1])
            body = json.loads(out.read(length))
            with self.cond:
                self.messages.append((time.perf_counter(), body))
                self.cond.notify_all()

    def send(self, message):
        data = json.dumps(message).encode()
        self.proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(data) + data)
        self.proc.stdin.flush()

    def notify(self, method, params):
        self.send({"jsonrpc": "2.0", "method": method, "params": params})

    def request(self, method, params):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        return self.next_id

    def wait(self, test, start):
        """The time from `start` to the first message after it that passes `test`, in ms."""
        with self.cond:
            while True:
                for at, m in self.messages:
                    if at >= start and test(m):
                        return (at - start) * 1000
                self.cond.wait(30)


def measure(minnow, root, runs, file=None):
    """Medians in ms of: open to diagnostics, edit to an answer, hover, definition, completion."""
    root = Path(root).resolve()
    modules = sorted((p for p in (root / "src").glob("m*.mn") if p.stem[1:].isdigit()),
                     key=lambda p: int(p.stem[1:]))
    target = root / "src" / file if file else modules[len(modules) // 2]
    uri = target.as_uri()
    text = target.read_text()
    lines = text.splitlines()
    # A call of another module's function.
    call = next(i for i, l in enumerate(lines)
                if ".check" in l or ("total = total +" in l and ".f" in l))
    col = lines[call].index(".") + 2
    results = {k: [] for k in KEYS}
    for _ in range(runs):
        c = Client([str(Path(minnow).resolve()), "lsp"], root)
        c.request("initialize", {"processId": None, "rootUri": root.as_uri(), "capabilities": {}})
        c.notify("initialized", {})
        published = lambda m: (m.get("method") == "textDocument/publishDiagnostics"
                               and m["params"]["uri"] == uri)
        start = time.perf_counter()
        c.notify("textDocument/didOpen", {"textDocument": {
            "uri": uri, "languageId": "minnow", "version": 1, "text": text}})
        results["open"].append(c.wait(published, start))
        # An edit (a new statement in the middle), then a request, which needs the new analysis.
        edited = lines[:]
        edited.insert(call + 1, "  sum = sum + 1;" if file else "  total = total + 1;")
        start = time.perf_counter()
        c.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": 2},
                                             "contentChanges": [{"text": "\n".join(edited)}]})
        rid = c.request("textDocument/hover", {"textDocument": {"uri": uri},
                                               "position": {"line": call, "character": col}})
        results["edit"].append(c.wait(lambda m: m.get("id") == rid, start))
        for name, method in [("hover", "textDocument/hover"),
                             ("definition", "textDocument/definition"),
                             ("completion", "textDocument/completion")]:
            start = time.perf_counter()
            rid = c.request(method, {"textDocument": {"uri": uri},
                                     "position": {"line": call, "character": col}})
            results[name].append(c.wait(lambda m: m.get("id") == rid, start))
        c.request("shutdown", None)
        c.notify("exit", None)
        c.proc.wait()
    return target.name, {k: statistics.median(v) for k, v in results.items()}


KEYS = ["open", "edit", "hover", "definition", "completion"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("minnow")
    parser.add_argument("dir")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--file", help="the file to open (default: a module in the middle)")
    args = parser.parse_args()
    name, medians = measure(args.minnow, args.dir, args.runs, args.file)
    print(f"`minnow lsp`, {name} open; medians of {args.runs} runs:\n")
    print("| open to diagnostics | edit to an answer | hover | definition | completion |")
    print("|---|---|---|---|---|")
    print("| " + " | ".join(f"{medians[k]:.0f} ms" for k in KEYS) + " |")


if __name__ == "__main__":
    main()
