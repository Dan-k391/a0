# a0 for Python

The Python package `a0`: A0's C API with ctypes, for languages written in Python. `docs/python.md`
describes it; [`examples/pebble`](examples/pebble) is a language written with it, and
[`tests`](tests) its tests (`cargo test -p a0-capi --test python`).

```
cargo build -p a0-capi
python examples/pebble/pebble.py run examples/pebble/programs/basics.pb
```
