#!/usr/bin/env python3
"""Coverage tooling: fail unless a raw rustc WASM module carries LLVM coverage
mapping sections (__llvm_covmap/__llvm_covfun) for llvm-cov to read."""
import sys


def leb128(data, index):
    result = shift = 0
    while True:
        byte = data[index]
        index += 1
        result |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return result, index


def main(path):
    data = open(path, "rb").read()
    if data[:4] != b"\0asm":
        raise SystemExit(f"{path}: not a WASM module")
    custom = {}
    index = 8
    while index < len(data):
        section = data[index]
        size, index = leb128(data, index + 1)
        end = index + size
        if section == 0:
            length, start = leb128(data, index)
            name = data[start:start + length].decode(errors="replace")
            custom[name] = custom.get(name, 0) + size
        index = end
    missing = [name for name in ("__llvm_covmap", "__llvm_covfun") if name not in custom]
    if missing:
        raise SystemExit(f"{path}: missing coverage sections {missing}; was it built instrumented?")
    print(" ".join(f"{name}={size}" for name, size in sorted(custom.items()) if name.startswith("__llvm")))


if __name__ == "__main__":
    main(sys.argv[1])
