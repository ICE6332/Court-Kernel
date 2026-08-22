# Court Kernel Engineering Roadmap

本文档把 Court Kernel 的工程路线固定为三段式推进。目标不是一开始完整从零写裸机 OS，而是先验证接口、对象模型和服务组合，再进入真正的 Root Court microhypervisor。

## 1. 路线 A：Linux-hosted research prototype

第一阶段在 Linux host 上模拟庭内核对象模型，优先验证架构是否好用。

映射关系：

```text
process/container      = Court
Unix domain socket     = Control Channel corridor
shared mmap            = Shared Ring corridor
cgroup/cpuset          = TimeBudgetCap / CpuSetCap 的弱模拟
VFIO/DPDK              = device corridor
FUSE/9P                = namespace prototype
```

第一阶段验收目标：

```text
庭划分是否合理
连廊 ABI 是否好用
namespace + capability 模型是否能表达真实服务
capability grant / delegate / revoke 是否能闭环
Court crash 是否能被 containment，而不是传播成全局崩溃
trace 是否能解释 corridor latency、peer-down、revoke 等事件
```

当前仓库先实现 MVP-0A：一个 in-process hosted object model。它不模拟 Linux 进程边界，只验证 Root Court 对象模型、capability、namespace、corridor、trace 和 fault containment 的语义。后续 MVP-0B 再拆成多进程 Linux prototype。

## 2. 路线 B：seL4 / Genode substrate prototype

第二阶段可把 Court 映射为 seL4 或 Genode 上的 component，借助已有 capability-based component architecture 验证更接近真实隔离的组件组合。

重点验证：

```text
capability delegation
component isolation
namespace routing
service composition
device driver / protocol stack / runtime environment 的庭化边界
```

路线 B 是对对象模型的交叉验证，不替代最终 Court Kernel。若路线 A 暴露了 Corridor ABI 或 namespace/capability 表达力问题，应先回到路线 A 修正，再进入路线 B。

## 3. 路线 C：自研 Root Court microhypervisor

第三阶段才进入真正庭内核。

核心组件：

```text
Root Court
VMX/EPT
IOMMU
APIC
capability kernel
namespace governor
court loader
corridor runtime
trace / health / policy substrate
```

进入路线 C 的前置条件：

```text
MVP-0 hosted prototype 已证明 open / revoke / peer-down / trace 语义
至少一个真实服务形态的 Net Court 或 Storage Court demo 已在路线 A 中跑通
路线 B 或等效 substrate 已验证 component isolation 与 capability delegation
Corridor ABI 已有版本化 IDL、状态机和失败语义
Root Court 的最小职责边界已经冻结
```

## 4. 当前状态与最近目标

MVP-0A 已完成 in-process object model。MVP-0B 已完成 Linux/WSL2 多进程 hosted prototype：

```text
Root process = Hosted Root Court
App process  = App Court
Net process  = Net Court
Unix domain socket = Control Channel corridor
shared mmap file   = Shared Ring corridor
```

MVP-0C 已完成：Root runner 由 manifest + policy 驱动。

```text
1. fixtures/packet-rx/manifest.json 声明 Courts、Corridors、ring 参数和 demo payload。
2. fixtures/packet-rx/policy.json 声明 grant/revoke/fault/peer-down 策略。
3. ck-root --manifest <path> --policy <path> --run-dir <path> 按声明启动 demo。
4. ck-root --demo packet-rx 是同一套 fixture 的编译内嵌副本。
5. policy.after 必须指向 MVP-0C pipeline 中严格更早的 phase。
6. trace 记录 lookup、open_denied、grant、open、send、recv、revoke、fault、peer-down、demo_done。
```

路线 A 下一步（尚未开始）：第二个服务形态（例如 Storage Court），或给 corridor 补显式状态机。进路线 B/C 的前置条件不变。

MVP-1 QEMU/UEFI bring-up 已在 Debian WSL2 上按 RFC-0001 §20.2 跑通：

```text
UEFI64 + Limine 10 加载 Root Court ELF（higher-half, long mode, HHDM paging）
自有 per-CPU GDT/TSS/IST（每核 ltr 0x18；IDT 256 门带真实向量号）
usable 区 bump allocator + HHDM 试写
x2APIC 周期 timer + ICR self/all-except-self IPI ping-pong
Limine MP 拉起 3 个 AP（-smp 4, -cpu max），isa-debug-exit 成功
```

MVP-2 第一刀已完成：Root Court 自建 4-level PML4，按 revision 3 语义映射内核 higher-half + restrictive HHDM，BSP/AP 都 `mov cr3` 切过去。页表策略尚未给 Courtlet 做独立地址空间。下一步才是加载 Court Image / 隔离域雏形 / corridor，不是 VMX。运行：`scripts/run-qemu.sh`。

### 已记账、尚未修的隐形依赖

这些不是现在的 bug，但顺序错了会直接炸。写在这里以免几周后忘掉。

1. **回收 bootloader-reclaimable 之前必须先换栈。**
   BSP 和所有 AP 仍跑在 Limine 分配的栈上，那些栈就在 reclaimable 区域。正确顺序：自建每 CPU 内核栈 → 切 RSP → 才能回收 Limine 内存。现在切 CR3 只是把 reclaimable 继续映射进自建 HHDM，没有收回。
2. **PTE NX 位依赖 `EFER.NXE`。**
   当前映射故意不设 NX。做 W^X 时必须先置 `IA32_EFER.NXE`，再往 PTE 写 NX（bit 63）；顺序反了是 reserved-bit `#PF`。
3. **Courtlet 独立地址空间需要共享内核高半区的 PML4 克隆入口。**
   现在的 `Mapper` 只有一份 PML4。给 Courtlet 建独立 CR3 时，应复制 Root 的内核 higher-half PML4 条目（index 511），不要为每个庭重走一遍内核映射。
