# Court Kernel

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

**Court Kernel**（庭内核，代号 Garden）是一个研究型 federated kernel 原型：把操作系统按功能切成并列的 **Court**（庭），由最小可信根 **Root Court** 负责 capability、命名空间、隔离和 **Corridor**（连廊）。

它**不是** Linux/POSIX ABI，也**还不是** VMX/EPT microhypervisor。当前目标是先把对象模型和隔离边界做对，再决定要不要走到自研 hypervisor。

This repository is a research prototype of a federated OS for heterogeneous x86. Hosted crates prove the object model on Linux; `crates/root-court` is a real UEFI kernel that boots under QEMU.

## Why

Monolithic kernels put networking, storage, GPU, and app runtimes in one privilege domain. Court Kernel treats each of those as a court: they discover each other by name, talk only over explicit corridors, and hold rights only as capabilities that Root Court can grant, attenuate, and revoke.

Design documents:

- [RFC-0001](Court-Kernel-RFC-0001.md) — architecture and object model
- [RFC-0002](Court-Kernel-RFC-0002.md) — Court Kernel ABI and Corridor ABI draft
- [Engineering roadmap](ENGINEERING-ROADMAP.md) — route A (hosted) → B (seL4/Genode) → C (microhypervisor)

## Status

| Stage | What it is | State |
|---|---|---|
| MVP-0A | `court-hosted`: in-process Root Court object model | done |
| MVP-0B | `court-hosted-linux`: multi-process Unix prototype (`ck-root` / `ck-app` / `ck-net`) | done |
| MVP-0C | `manifest.json` + `policy.json` driven demo | done |
| MVP-1 | QEMU/UEFI Root Court bring-up | done |
| MVP-2 | Trusted courtlets: own CR3, per-CPU stacks, shared ring, cap revoke | in progress (same-ring stubs; no Court Image loader yet) |
| Route B | seL4 / Genode substrate | not started |
| Route C | Root Court microhypervisor (VMX/EPT/IOMMU) | not started |

Hosted demo: App Court sends on `/court/net0/packet/rx`, Net Court receives, then revoke / fault / peer-down are recorded in `trace.ndjson`.

QEMU demo: Root Court boots on 4 CPUs, switches onto owned page tables and kernel stacks, runs two trusted courtlets over a shared ring, and denies send after revoke. Success is serial `BOOT_OK` and QEMU exit status 33 (`isa-debug-exit`).

## Layout

```text
Court-Kernel/
  Court-Kernel-RFC-0001.md    architecture and object model
  Court-Kernel-RFC-0002.md    ABI and Corridor ABI draft
  ENGINEERING-ROADMAP.md      route A → B → C
  fixtures/packet-rx          canonical MVP-0C manifest + policy
  crates/court-hosted         in-process object model (`unsafe` forbidden)
  crates/court-hosted-linux   Unix multi-process mapping + ck-* binaries
  crates/root-court           no_std Root Court kernel (UEFI / Limine)
  boot/limine.conf            Limine boot entry
  scripts/run-qemu.sh         build ISO and boot QEMU
```

## Design in one page

- **Names discover, capabilities authorize.** Looking up `/court/net0/packet/rx` does not grant `open`.
- **CSpace lives in Root Court.** Children hold wire cap ids; forged ids fail as `BadCap`.
- **Corridors are explicit.** Control plane is a Unix domain socket (hosted) or a Root-mediated channel (bare metal); data plane is a shared SPSC ring.
- **Revoke and crash are contained.** Delegated caps revoke down the derivation tree. A faulted Net Court does not take down App Court; later send returns `PeerDown`.

RFC-0002 is the future ABI (`ck_cap_invoke`, five corridor transports, C/Rust layout). The hosted prototype does not implement that ABI yet.

## Toolchain

Rust **1.98.0**, edition 2024, pinned by `rust-toolchain.toml`. `rustup` installs that compiler, `rustfmt`, `clippy`, `llvm-tools-preview`, and `x86_64-unknown-none` when you enter the repo.

## Build and test

```bash
cargo build --workspace
cargo test  --workspace
```

Unix end-to-end demo (Linux or WSL2):

```bash
cargo test -p court-hosted-linux --test mvp0b_demo
```

Packet-rx fixture (Linux / Debian WSL2):

```bash
cargo run -p court-hosted-linux --bin ck-root -- --demo packet-rx --run-dir /tmp/ck-run
```

Or the checked-in files (same content as `--demo packet-rx`):

```bash
cargo run -p court-hosted-linux --bin ck-root -- \
    --manifest fixtures/packet-rx/manifest.json \
    --policy   fixtures/packet-rx/policy.json \
    --run-dir  /tmp/ck-run
```

`ck-app` and `ck-net` are spawned by `ck-root`. On Windows, `cargo test --workspace` still builds the portable protocol / manifest / object-model tests; the multi-process demo is `cfg(unix)` only.

## Bare-metal boot (QEMU + UEFI)

Linux or Debian WSL2. Install `qemu-system-x86`, OVMF, and `xorriso`, then:

```bash
bash scripts/run-qemu.sh
```

This builds `root-court` for `x86_64-unknown-none`, wraps it in a Limine UEFI ISO, and boots QEMU with serial on stdio (`-cpu max` is required for x2APIC). The kernel:

1. loads its own GDT/IDT and a bump allocator from the Limine usable map
2. switches to its own 4-level page tables (kernel higher-half + Limine revision-3 HHDM)
3. moves every CPU onto an owned kernel stack
4. enables x2APIC and ping-pongs ICR IPIs across 4 CPUs
5. runs two trusted courtlets (cloned CR3, shared ring, cap revoke)

## What this is not

- Not a Linux replacement, container runtime, or POSIX personality
- Not a production capability microkernel
- Not yet a hypervisor: no VMX, EPT, or IOMMU
- Courtlet entries still live in the kernel higher-half; independent Court Images are next, not Route C

## License

Copyright 2026 ICE6332.

Licensed under the [Apache License, Version 2.0](LICENSE).
