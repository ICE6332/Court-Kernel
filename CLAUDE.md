# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project context

Court Kernel（庭内核，代号 Garden）是一个研究型 OS 架构的 federated kernel 原型。设计文档：

- `Court-Kernel-RFC-0001.md` — 总体架构、对象模型（Court / Capability / Namespace / Corridor / Placement）。
- `Court-Kernel-RFC-0002.md` — Court Kernel ABI 与 Corridor ABI 草案（`ck_cap_invoke` / `ck_msg` / `ck_boot_info` / 5 类 corridor transport / IDL）。这是**未来**的工程合约，不是当前 hosted prototype 必须立刻满足的接口。
- `ENGINEERING-ROADMAP.md` — 三段式推进（路线 A → B → C）与 MVP 状态。

仓库同时有 **路线 A — Linux-hosted research prototype**（`court-hosted*`）和 **MVP-1 Root Court bring-up**（`crates/root-court`，QEMU/UEFI 可启动）。hosted 线用来验证对象模型、capability、namespace、corridor、trace 和 fault containment；裸机线刚完成 GDT/IDT、bump allocator、x2APIC timer 和 ICR IPI。RFC-0001 中的 VMX/EPT/IOMMU 和 RFC-0002 的 `vmcall` / `vmmcall` 都尚未启动，不要把那一层的问题混进 hosted prototype——RFC-0002 §2 把 hosted 阶段定义为 "Stage A: Rust/C 内部函数调用模拟 ck_cap_invoke"。

阶段命名（提交历史和 trace 事件名都会出现）：
- **MVP-0A** = `court-hosted` crate，in-process 对象模型。
- **MVP-0B** = `court-hosted-linux` crate，多进程 Unix host 原型。
- **MVP-0C** = `ck-root` 由 `fixtures/packet-rx/manifest.json` + `policy.json` 驱动；`--demo packet-rx` 是同一套 fixture 的编译内嵌副本。policy `after` 必须指向 pipeline 中更早的 phase。

Toolchain is pinned in `rust-toolchain.toml` to **Rust 1.98.0** / edition 2024 (`x86_64-unknown-none` + `llvm-tools-preview` included). Hosted code may use 1.95+ language features (`cfg_select!`, match `if let` guards) and should not assume a Windows global toolchain older than that.

Bare-metal Root Court lives in `crates/root-court` (`#![no_std]`, Limine protocol implemented in-tree so it stays on stable). It now owns per-CPU GDT/TSS, a 256-vector IDT, a usable-map bump allocator, its own 4-level page tables (Limine revision-3 HHDM semantics), x2APIC timer, and ICR IPI. Do not reclaim bootloader-reclaimable while CPUs still use Limine stacks; do not set PTE NX before `EFER.NXE`; Courtlet page tables should clone the kernel's high-half PML4 entry. See `ENGINEERING-ROADMAP.md` "已记账、尚未修的隐形依赖". Do not mix VMX/EPT work into `court-hosted`. Boot with `scripts/run-qemu.sh` from Linux/WSL2 (`-cpu max` is required for x2APIC). `cargo test --workspace` uses `default-members` and does **not** build the kernel image; use `cargo build -p root-court --release --target x86_64-unknown-none`.

## Build, test, run

```bash
cargo build --workspace
cargo test  --workspace                                    # core unit tests
cargo test  -p court-hosted-linux --test mvp0b_demo        # end-to-end Unix demo (Linux/WSL2 only)
cargo test  -p court-hosted lookup_without_cap_does_not_authorize_open  # single test
```

跑 demo 二进制：

```bash
# 内置 fixture
cargo run -p court-hosted-linux --bin ck-root -- --demo packet-rx --run-dir /tmp/ck-run

# 与 --demo packet-rx 等价的磁盘 fixture
cargo run -p court-hosted-linux --bin ck-root -- \
    --manifest fixtures/packet-rx/manifest.json \
    --policy   fixtures/packet-rx/policy.json \
    --run-dir  /tmp/ck-run
```

`ck-app` 和 `ck-net` 不应直接由人手启动——`ck-root` 会 spawn 它们，通过环境变量 `CK_APP_BIN` / `CK_NET_BIN` 指定可执行路径（测试里用 `CARGO_BIN_EXE_*` 注入），找不到则 fallback 成 `ck-root --court-role <role>` 把同一 binary 重新执行。

### 平台限制

- `court-hosted-linux` 的 Unix 实现（`app`/`net`/`root`/`control`/`shm_ring` 模块和三个 `ck-*` 二进制）都用 `#[cfg(unix)]` 包起来，依赖 Unix domain socket、mmap、libc。**Windows host 上 `cargo build` / `cargo test` 仍能通过**（只编译 `protocol` 与 `manifest`），但 demo 和 e2e 测试只能在 Linux/WSL2 跑。
- 用户的开发机是 Windows，跨平台改动需要保持这个 cfg 切分；不要把 Unix-only 调用泄漏到 `protocol.rs` / `manifest.rs` / `lib.rs` 里。
- `court-hosted` crate 顶部声明 `unsafe_code = "forbid"`。`shm_ring.rs` 是当前唯一允许 `unsafe` 的地方（mmap + 原子 SPSC ring）；新加的 unsafe 必须配 `// SAFETY:` 注释，沿用现有风格。

## High-level architecture

### 两层 crate 结构

`crates/court-hosted` 是纯逻辑的 Root Court 对象模型，所有状态在 `HostedRoot` 一个结构体里（`HashMap<CourtId, Court>` + cspace + cap registry + namespace + corridors + trace）。它不知道进程、socket、mmap，只暴露 `create_court / create_corridor / lookup / grant_corridor_cap / open / send / recv / revoke / fault_court / trace`。MVP-0A 单元测试验证 capability、revocation、peer-down 这些核心语义。

`crates/court-hosted-linux` 把 MVP-0A 模型映射到 Unix 多进程世界：

| RFC 概念 | hosted prototype 实现 |
|---|---|
| Root Court | `ck-root` 进程，持有唯一一份 `HostedRoot` |
| App / Net Court | `ck-app` / `ck-net` 子进程 |
| Control Channel corridor | Unix domain socket + JSON-lines (`control.rs`) |
| Shared Ring corridor | mmap 文件 + SPSC 原子 ring (`shm_ring.rs`) |
| Capability | `WireCap { id, rights }` 在 wire 上传输；真实 `CapId` 只存在 root 进程 |
| Trace | `trace.ndjson`（每行一个 `WireTrace`）写在 `--run-dir` 下 |

子进程**不**直接持有 core `CapId`——它们只拿到 `WireCap.id`，每次 `Open` 都把 wire id 寄回 root，root 在 `caps_by_id` 里查回真实 `CapId` 再调 `HostedRoot::open`。这是关键的隔离点：cspace 永远在 root 一侧，子进程伪造 cap id 会被 `BadCap` 拒绝。

### Demo 状态机

`root::run_manifest_demo` 是当前唯一的编排入口。MVP-0C 的 court/corridor/grant/revoke/fault/peer-down **集合**来自 manifest + policy，执行顺序仍是固定 pipeline（不是通用事件循环）。`policy.*.after` 由 `manifest::validate` 对照 `PHASE_ORDER` 检查，不能指向当前动作或更晚的 phase。

固定顺序：

1. 读 manifest/policy → `validate()` → 建 courts、corridors、shared rings → bind namespace。
2. spawn `ck-app` / `ck-net` 子进程，accept 它们的 `Hello` 并回 `HelloAck { demo }`。
3. App 先做一次**不带 cap 的 Open**，验证 `NoRight` 路径并写 `open_denied` trace。
4. 按 `policy.grants` 顺序发 `Grant` → 等子进程 `Open(cap)` → 回 `OpenResult { ring_info }`。
5. App 通过 mmap ring `send`，Net 通过 mmap ring `recv`，分别上报 `send` / `recv` trace。
6. 按 `policy.revokes` 撤销 cap，子进程在收到 `Revoke` 后再尝试一次 `ring.send` 验证 `Revoked`。
7. 按 `policy.faults` 给目标 court 发 `Fault`，子进程 `exit(42)`；root 调 `fault_court` 并断言子进程退出码是 42。
8. 按 `policy.peer_down` 通知幸存方，等它回 `DemoDone`。

修改 demo 流程时要同时改 `crates/court-hosted-linux/tests/mvp0b_demo.rs` 里 `assert_event` 期望的 trace 事件名/状态，否则 e2e 会断在 trace 断言上。

### Wire protocol

`protocol.rs` 定义所有跨进程消息（`WireMessage` 是 tag = "type" 的 enum，serde_json 序列化为 newline-delimited JSON）。新增字段优先用 `Option<...>` 保持向后兼容；新增消息变体要同时更新：root 的状态机分支、`ck-app` / `ck-net` 的接收循环、e2e 测试期望，以及 trace 事件名（trace 名和 `WireMessage` 类型分两条枚举，不要假设它们对齐）。

### 命名空间路径

仓库内固定 demo 路径常量是 `/court/net0/packet/rx`（见 `protocol::PACKET_RX_PATH` 和 RFC §5）。manifest 里的 corridor `path` 同时充当 namespace 路径和 ring 文件名 stem（`root::ring_file_name` 取最后两段拼成 `<parent>-<leaf>.ring`）。

### Rights 模型

`Rights` 是 bitflag。`court-hosted` 内部用 `Rights` 类型，跨进程时序列化成 `WireRights { bits: u64 }`。新加 right 要同步：`court-hosted::Rights` 常量、`protocol::WireRights::to_rights`、`manifest::parse_rights`，否则 manifest 里写的新 right 会被 reject。

## 当前 prototype 与 RFC-0002 ABI 的已知差距

下面这些不是 bug，是 hosted prototype 阶段的有意简化。改动相关代码时知道差距在哪即可，不要顺手"对齐 RFC"——真正的对齐发生在引入 `abi/` crate 与 Stage B/C 之后（参考 RFC-0002 §28 的目标目录布局：`abi/include/ck/abi.h` + `abi/rust/ck-abi/src/lib.rs` + 双语 layout tests）。

- **Rights 集合不全**。`court-hosted::Rights` 当前只有 `READ/WRITE/SEND/RECV/DELEGATE/REVOKE/OBSERVE`。RFC-0002 §8.2 还定义了 `EXECUTE/MAP/MINT/CONFIGURE/BIND/SCHEDULE/SIGNAL/WAIT/TRANSFER/ADMIN`。新增 manifest right 时只能从已有这 7 种里挑，否则 `parse_rights` 会拒绝。
- **错误码不是 ABI 形态**。当前 `CkError::BadCap/NoRight/Revoked/...` 是 Rust enum；RFC-0002 §9 定义的是 `ck_status_t` 数字错误码（`CK_ERR_NOENT/ACCESS/BADTYPE/REVOKED/ABI/...`），且和当前 enum 不是一对一映射（例如 `BadCap` 对应 `NOENT` 还是 `BADTYPE` 取决于上下文）。`WireStatus` 是当前 wire 层的折中。
- **没有 `ck_cap_invoke` 单一入口**。当前 root 直接调用 `HostedRoot::lookup / open / send / recv / revoke` 等具名方法；RFC-0002 §10 规定所有 root 操作都走 `ck_cap_invoke(target, op, msg, flags)` 加 op-id 分发。引入 `abi/` 之后才会出现这个 dispatch 层。
- **Corridor 没有 RFC §15.2 的状态机**。当前只有"是否 peer down"的隐式判断，没有 `INIT/READY/DRAINING/REVOKING/REVOKED/DEAD` 显式状态。`fault_court` 的传播也是直接查 `courts` map，不是通过状态转换。
- **Shared ring 与 RFC §17 baseline 有偏差**：
  - 当前 magic = `0x434b_5247`（4 字节 "CKRG"）；RFC §17.2 是 `0x434b52494e473032`（8 字节 "CKRING02"）。
  - 当前 capacity 用 `producer % capacity`，**不要求**是 2 的幂；RFC §17.8 要求 power-of-two 并用 `index & (capacity - 1)`。
  - 当前 ring header 没有 `dropped_count` / `error_count` / `producer_event_idx` / `consumer_event_idx` / 8 个 reserved——所以也没有 RFC §17.9 的 notification suppression。
  - 当前 ring 是 length-prefixed byte slot；RFC §17.4 的 baseline 是 `ck_ring_desc`（region_id + offset + len + seq + meta），数据本身放在 bulk region。
- **无 protocol ID / 版本协商**。RFC-0002 §21 要求每个 corridor 上层协议带 128-bit protocol ID 与 major/minor/patch；当前 demo 直接绑死 `/court/net0/packet/rx` 路径，没有协议层。
- **Wire trace ≠ ABI trace**。当前 `WireTrace { event: String, court, path, status, len, detail }` 是 demo 的人读 NDJSON；RFC-0002 §25.1 的 `ck_trace_event { timestamp_ns, source_court_id, corridor_id, event_type: u32, arg0..arg3 }` 是固定 layout 的二进制事件。两者都会保留，但不要假设字段名能直接搬。

## 联邦内核双护栏（新会话开工前必过）

庭内核是 federated kernel，不是宏内核，也不是微内核变种。细则见 `.cursor/rules/federated-kernel-gates.mdc` 和 RFC-0001 §6.1。

当前审计：`crates/root-court` 职责仍是 bring-up TCB，**没有**协议栈/FS/POSIX。app/net 已是独立 Court Image，由 Root Court 加载到 lower-half。禁止 VMX，禁止通用 IPC syscall，禁止把业务逻辑写回 Root Court。

## 写代码时的约束

- 修改对象模型语义（cap derivation、revoke 传播、peer-down 检测）务必同步更新 `court-hosted/src/lib.rs` 的 `tests` 模块——它们是当前唯一覆盖语义边界的回归测试。
- Trace 是 demo 的可观测性合约，不要随手改 trace 事件名或 status 字符串；e2e 测试逐字匹配。
- Demo 里若要 spawn 新角色（除了现有 app/net），需要扩 `CourtRole` enum、新增对应 binary、扩 `court_command` 的环境变量分支，并加 `CARGO_BIN_EXE_*` 注入到 e2e。
- `unsafe_code = "forbid"` 是 `court-hosted` 的硬性约束。如果非要写裸指针/SIMD，应放进 `court-hosted-linux` 或新 crate，并写完整 `// SAFETY:` 注释。

## 远程 Windows x86 主机

开发端是 Mac，运行端是原生 x86 Windows 主机。SSH 代称是 **`court-kernel`**（只写在本机 `~/.ssh/config`，不要把 HostName/User/密钥写进仓库）。不要用其它项目的 SSH Host 名。连接、同步和 WSL2/QEMU 步骤见 `docs/remote-development.md`。

同步前先看远程 `git status --short`。有未提交改动就停，不要 `reset --hard` / `clean` / `stash`。Mac 推到 `origin/main` 后，远程只允许 `git pull --ff-only origin main`。Windows 上需要 bash 时用 `C:\Program Files\Git\bin\bash.exe`，不要用 PATH 里的 WSL `bash`。
