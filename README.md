# Court Kernel

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

庭内核（Court Kernel，代号 Garden）是一个研究型 **federated kernel** 原型：操作系统按功能切成并列的 Court（庭），由一个最小可信根 Root Court 负责 capability、命名空间、隔离和连廊（Corridor）。

当前仓库实现的是 **路线 A：Linux-hosted research prototype**。它还不是可启动的裸机内核，也不提供 POSIX/Linux ABI。它用来在 host 上验证对象模型、capability、namespace、corridor、trace 和 fault containment。

This repository is a research prototype of a federated OS architecture for heterogeneous x86. The live code is a Linux-hosted object-model simulator, not a bootable kernel.

## Status

| Stage | What it is | State |
|---|---|---|
| MVP-0A | `court-hosted`: in-process Root Court object model | done |
| MVP-0B | `court-hosted-linux`: multi-process Unix prototype (`ck-root` / `ck-app` / `ck-net`) | done |
| MVP-0C | manifest.json + policy.json driven demo | done |
| Route B | seL4 / Genode substrate | not started |
| Route C | bare-metal Root Court microhypervisor (VMX/EPT/IOMMU) | not started |

The built-in demo is a fake packet path: App Court sends on `/court/net0/packet/rx`, Net Court receives, then revoke / fault / peer-down are exercised and recorded in `trace.ndjson`.

## Layout

```text
Court-Kernel/
  Court-Kernel-RFC-0001.md   architecture and object model
  Court-Kernel-RFC-0002.md   Court Kernel ABI and Corridor ABI draft
  ENGINEERING-ROADMAP.md     route A → B → C
  fixtures/packet-rx         canonical MVP-0C manifest + policy
  crates/court-hosted        in-process object model (no unsafe)
  crates/court-hosted-linux  Unix multi-process mapping + ck-* binaries
```

## Toolchain

Pinned to **Rust 1.98.0** via `rust-toolchain.toml` (`edition = "2024"`, `rust-version = "1.98"`). `rustup` will install that compiler when you enter the repo.

## Build and test

```bash
cargo build --workspace
cargo test  --workspace
```

End-to-end Unix demo (Linux or WSL2 only):

```bash
cargo test -p court-hosted-linux --test mvp0b_demo
```

Run the packet-rx fixture (Linux / Debian WSL2):

```bash
cargo run -p court-hosted-linux --bin ck-root -- --demo packet-rx --run-dir /tmp/ck-run
```

Or from the checked-in files (same content as `--demo packet-rx`):

```bash
cargo run -p court-hosted-linux --bin ck-root -- \
    --manifest fixtures/packet-rx/manifest.json \
    --policy   fixtures/packet-rx/policy.json \
    --run-dir  /tmp/ck-run
```

`ck-app` and `ck-net` are spawned by `ck-root`. On Windows, `cargo test --workspace` still builds the portable protocol/manifest/object-model tests; the multi-process demo is `cfg(unix)` only.

## Design in one page

- **Names discover, capabilities authorize.** Lookup of `/court/net0/packet/rx` does not grant `open`.
- **CSpace lives in Root Court.** Child processes hold wire cap ids; forged ids fail as `BadCap`.
- **Corridors are explicit.** Control plane is a Unix domain socket; data plane is a shared mmap SPSC ring.
- **Revoke and crash are contained.** Delegated caps revoke down the derivation tree. A faulted Net Court does not take down App Court; later send returns `PeerDown`.

RFC-0002 is the future ABI (`ck_cap_invoke`, five corridor transports, C/Rust layout). The hosted prototype deliberately does not implement that ABI yet.

## License

Copyright 2026 ICE6332.

Licensed under the [Apache License, Version 2.0](LICENSE).
