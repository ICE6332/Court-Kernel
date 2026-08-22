# Court Kernel RFC-0001

**标题**：庭内核总体架构、对象模型与最小工程实现草案  
**状态**：Draft 0.1  
**日期**：2026-04-25  
**语言**：中文  
**目标平台**：经典现代 x86-64；优先覆盖 Intel Hybrid / AMD heterogeneous-class core 平台  
**实现语言**：Rust + C + x86-64 Assembly  
**适用范围**：研究型内核、工程原型、面向异构多核与功能切片的 OS 架构设计  

---

## 0. 摘要

Court Kernel，中文名“庭内核”，是一种面向现代 x86-64 异构多核平台的操作系统架构。它不把 OS 组织成一个统一的宏内核，也不强制所有服务以同一种微内核 IPC 模式运行，而是把系统拆分为若干个并列协作的“庭”（Court）。每个庭是一个功能垂直切片，拥有自己的内部结构、局部调度策略、安全模型和最小运行时。庭与庭之间通过显式的“连廊”（Corridor）通信，所有资源、接口、能力与观测点被组织在统一命名空间中。

本 RFC 将庭内核定义为：

> 一个以功能庭为组织单元，以 capability 为权限根，以统一命名空间为治理面，以 x86 VMX/EPT/VT-d/APIC/HFI 等机制为硬件承载的 federated kernel architecture。

本 RFC 的核心工程约束是：

1. **庭核不得作为多个裸 ring 0 子内核并列运行。** 这会导致任何一个庭核都拥有破坏全系统的物理权限，削弱 capability 的实际意义。
2. **系统必须存在一个最小可信根 Root Court。** Root Court 运行在最高物理特权层，负责能力、命名、隔离、放置、加载、连廊建立和撤销。
3. **功能庭 Courtlet 默认运行在受控执行域中。** x86 原型优先采用 VMX non-root + EPT 隔离；早期 trusted bring-up 阶段可以临时使用同 ring 模式，但不得视为最终安全模型。
4. **路径名只负责发现，不代表权限。** 所有实际操作权限必须由 capability 表达。
5. **连廊是庭间 ABI 的核心。** 庭内部可自由组织，但跨庭接口必须显式、稳定、可观测、可治理。

本 RFC 同时确定第一版实现语言策略：

- **Rust** 是默认系统实现语言，用于 Root Court 的 capability、namespace、placement、corridor、loader、policy、trace 等核心逻辑，也用于大多数 Courtlet runtime。
- **C** 用于受限兼容层、C ABI、少量硬件/固件接口、迁移期驱动适配和不能合理用 Rust 表达的外部边界。
- **x86-64 Assembly** 只用于启动、模式切换、AP trampoline、中断入口/出口、上下文切换、VMX transition、特权指令薄封装等不可避免区域。

---

## 1. 术语与规范性语言

本文使用 RFC 风格的规范性措辞：

- **MUST / 必须**：实现必须满足，否则不符合 RFC-0001。
- **SHOULD / 应该**：强烈建议满足；不满足时必须给出工程理由。
- **MAY / 可以**：允许实现选择。
- **MUST NOT / 不得**：实现不得采用。

### 1.1 核心术语

| 术语 | 定义 |
|---|---|
| Court / 庭 | 一个功能垂直切片，是庭内核的基本组织单元。 |
| Courtlet / 庭核或庭运行时 | 某个庭在具体核心或执行域中运行的最小内核/运行时镜像。 |
| Root Court / 枢庭 | 唯一最高可信根，负责硬件资源、隔离、能力、命名、加载、连廊和放置。 |
| Corridor / 连廊 | 庭与庭之间的显式通信接口与传输对象。 |
| Capability / 能力 | 不可伪造的授权凭证，表示对某对象的一组操作权限。 |
| Namespace / 命名空间 | 用于发现资源、服务、连廊、能力入口和观测点的树状或图状命名系统。 |
| Placement / 放置 | 将庭或庭核映射到 CPU、核心类型、NUMA 节点、设备邻近性和时间预算上的决策。 |
| Court Image / 庭镜像 | 可被 Root Court 加载的庭核或运行时镜像。 |
| CSpace / 能力空间 | 某个庭可访问的 capability 集合。 |
| Corridor ABI / 连廊 ABI | 跨庭通信对象的调用、传输、权限、观测和撤销规则。 |
| Trusted Bring-up | 早期开发阶段的受信同 ring 原型，不具备最终隔离语义。 |

---

## 2. 设计目标

Court Kernel 的第一版目标不是替代 Linux，也不是追求完整 POSIX 兼容，而是建立一种可以被工程验证的新 OS 组织方式。

### 2.1 架构目标

1. **功能切片**：OS 由多个功能庭组成，每个庭服务一类相关需求，例如网络、存储、渲染、安全、应用运行时、命名、策略等。
2. **异构感知**：庭可以根据负载特性被映射到 P-core、E-core、classic core、dense core、NUMA locality 或独占核心。
3. **最小权限**：物理位置决定启动和硬件特权，但逻辑权限必须由 capability 决定。
4. **统一命名**：资源、连廊、服务、观测点和策略入口必须可在统一命名空间中发现。
5. **显式连廊**：庭之间不得通过隐式共享全局状态协作，必须通过显式 corridor 或显式共享 capability 协作。
6. **局部结构自由**：每个庭内部可以是分层、事件驱动、协程、数据面、library OS、微内核服务、状态机或专用 runtime。
7. **可演化**：庭内部可以替换，跨庭连廊契约保持稳定。

### 2.2 工程目标

1. 在 QEMU/KVM 上完成最小原型。
2. 在真实 x86-64 裸机上完成 Root Court bring-up。
3. 支持多核心启动、IPI、x2APIC timer、基础页表、物理内存分配。
4. 支持 capability mint/delegate/revoke。
5. 支持 namespace lookup 与 capability-mediated open。
6. 支持至少两种 corridor transport：control channel 与 shared ring。
7. 支持 Court Image 加载。
8. 支持 VMX/EPT 隔离的 Courtlet。
9. 支持至少一个网络庭 demo。
10. 支持基本 trace 与 health monitor。

---

## 3. 非目标

Court Kernel RFC-0001 明确不承诺以下内容：

1. 不承诺通用性能优于 Linux。
2. 不承诺第一版完整 POSIX 或 Linux ABI 兼容。
3. 不承诺所有 OS 服务都拆成用户态进程。
4. 不承诺所有功能都用同一种 IPC 机制。
5. 不承诺 E-core 天然抗侧信道。
6. 不承诺 namespace 本身就是访问控制机制。
7. 不承诺第一版可形式化验证，但架构必须为后续验证保留空间。
8. 不承诺支持所有 x86 设备，只优先支持可隔离、可枚举、可观测的设备路径。
9. 不承诺在无 IOMMU 的平台上提供强 DMA 隔离。
10. 不承诺 Root Court 可以容忍自身被攻破；Root Court 是最小可信根。

---

## 4. 设计依据与相关系统

Court Kernel 借鉴并重组了多条 OS 研究路线，但不等同于其中任何一个系统。

### 4.1 Plan 9

Plan 9 提供了统一命名空间与协议化资源访问的启发。Court Kernel 采用类似“资源可命名、接口可挂载、每个执行环境可拥有自己的 namespace view”的思想，但不把路径名视为权限。

Court Kernel 的原则是：

```text
名字负责发现。
能力负责授权。
连廊负责传输。
观测负责治理。
```

### 4.2 seL4 与 capability 系统

seL4 提供了高保证 capability microkernel 的重要参考。Court Kernel 借鉴 CSpace、typed capability invocation、能力派生和最小可信根思想，但不强制所有 OS 服务都按传统微内核服务模型运行。

Court Kernel 的差异是：

```text
seL4: capability microkernel + user-level components
Court Kernel: root capability court + isolated function courts + typed corridors
```

### 4.3 Fuchsia / Zircon

Zircon 的 handle rights 和 Fuchsia component namespace 对 Court Kernel 的“名字与权限分离”非常有参考价值。Court Kernel 采用类似的基本原则：对象引用与 rights 绑定在 capability/handle 中，namespace 只提供发现和路由入口。

### 4.4 Barrelfish

Barrelfish 的 multikernel 模型证明 OS 可以由多个内核节点组成，并通过显式通信协调。Court Kernel 接受“显式通信”和“多内核镜像”的思路，但把组织单位从“硬件核心”推进为“功能庭”。

```text
Barrelfish: core -> OS node
Court Kernel: function court -> courtlet image -> mapped onto one or more cores
```

### 4.5 Exokernel / Library OS

Exokernel 提供“保护和管理分离”的思想，Library OS 提供“功能按需裁剪”的实现路线。Court Kernel 接受这些思想，但不把所有策略都外移到应用。庭可以拥有自己的局部策略，但资源根权限仍由 Root Court capability 管控。

### 4.6 Arrakis / IX / Dune

Arrakis、IX 与 Dune 证明了现代硬件虚拟化、IOMMU、设备队列和数据面专用执行模型可以显著减少传统内核在高性能 I/O 路径上的干预。Court Kernel 将这种“控制面/数据面分离”推广为 corridor 类型之一，而不是只服务网络服务器。

---

## 5. 平台基底

RFC-0001 的目标平台是经典现代 x86-64。实现必须优先支持以下机制，缺失机制可以通过 fallback 降级，但安全语义必须明确降级。

### 5.1 最小硬件要求

```text
x86-64 long mode
UEFI boot
APIC 或 x2APIC
多核心启动能力
4-level paging；可选 5-level paging
CPUID topology enumeration
TSC / invariant TSC；如不可用则使用替代 timer
```

### 5.2 强隔离推荐要求

```text
Intel VMX + EPT 或 AMD SVM + NPT
Intel VT-d 或 AMD-Vi IOMMU
Interrupt remapping
MSI-X
PCID / INVPCID
```

### 5.3 异构调度推荐要求

```text
Intel HFI / Thread Director hint
AMD HFI hint
CPU capacity / efficiency hint
NUMA / cache topology
thermal / power telemetry
```

### 5.4 可选安全与性能机制

```text
CET
PKU
RDT/CAT
page coloring
SGX/TDX/SEV-SNP 类执行环境，作为后续扩展而非 RFC-0001 基线
```

---

## 6. 总体架构

Court Kernel 由 Root Court 与多个功能庭组成。

```text
┌─────────────────────────────────────────────────────────────┐
│                  Application / Library OS / Runtime          │
└───────────────▲──────────────────────▲──────────────────────┘
                │                      │
         App Court / POSIX Court   Render / Crypto / ML Court
                │                      │
        ┌───────┴────────┐     ┌───────┴────────┐
        │    Courtlet     │     │    Courtlet     │
        │ local runtime   │     │ local runtime   │
        │ local policy    │     │ local policy    │
        └───────▲────────┘     └───────▲────────┘
                │ Corridor             │ Corridor
┌───────────────┴──────────────────────┴──────────────────────┐
│                         Root Court                           │
│ cap registry | namespace root | placement | EPT | IOMMU      │
│ APIC | IRQ caps | device caps | trace | policy | health       │
└───────────────────────────────────────────────────────────────┘
                │
┌───────────────┴──────────────────────────────────────────────┐
│       x86-64 hardware: P/E cores, caches, PCIe, NIC, NVMe     │
└───────────────────────────────────────────────────────────────┘
```

### 6.1 Root Court

Root Court 是唯一最高可信根。它必须尽可能小，且不得吸收普通 OS 服务。

Root Court 必须负责：

```text
硬件资源枚举
物理内存根所有权
capability mint/delegate/revoke
namespace root 与 namespace view 管理
Court Image 加载
EPT/NPT 管理
IOMMU domain 管理
APIC/IRQ/MSI-X routing
CPUSet 与 TimeBudget 管理
Court placement
Corridor 建立、观测和撤销
health monitor 与 crash containment
trace endpoint 与审计日志
```

Root Court 不得负责：

```text
完整文件系统
TCP/IP 协议栈
GUI
复杂用户态兼容层
通用应用运行时
数据库、Web 服务或业务服务
可被庭化的复杂驱动逻辑
```

### 6.2 Court

Court 是功能组织单元，不是进程，不是 VM，也不必等同于一个核心。

一个 Court 可以：

```text
只运行在一个核心上
跨多个核心运行
与其他低负载 Court 共享核心
拥有多个 Courtlet 镜像
拥有自己的调度器、事件循环或数据面 runtime
暴露多个 corridor
拥有自己的 namespace view
```

### 6.3 Courtlet

Courtlet 是 Court 的具体执行实体。一个 Court 可以包含一个或多个 Courtlet。

Courtlet 默认必须运行在受控执行域中：

```text
首选：VMX non-root + EPT
可选：SVM guest + NPT
早期原型：trusted same-ring courtlet，仅用于 bring-up
```

Courtlet 启动时只获得 Root Court 注入的 bootstrap capability。除非持有相应 capability，Courtlet 不得访问其他内存、设备、IRQ、命名空间绑定或 corridor。

---

## 7. 实现语言策略

Court Kernel 使用 Rust、C、x86-64 Assembly 混合实现。语言边界是安全边界的一部分，必须被明确记录和审计。

### 7.1 Rust

Rust 是默认实现语言。Root Court 的大多数逻辑必须使用 Rust 编写。

Rust 使用策略：

```text
#![no_std]
core 优先
alloc 只在完成可信 allocator 后启用
panic = abort
禁止 unwinding
默认禁止浮点
默认禁止动态全局初始化
默认禁止未审计 unsafe
```

Root Court Rust 模块建议包括：

```text
capability
namespace
corridor
placement
loader
memory object model
interrupt object model
device object model
policy
trace
health
manifest parser
court registry
```

Rust 的 `unsafe` 必须集中在少数边界模块中，例如：

```text
arch::x86_64::msr
arch::x86_64::vmx
arch::x86_64::paging
arch::x86_64::apic
arch::x86_64::iommu
arch::x86_64::interrupt
ffi
mmio
```

每个 `unsafe` 模块必须包含：

```text
Safety Contract
Caller obligations
Callee guarantees
Aliasing model
Lifetime model
Interrupt/reentrancy assumptions
Memory ordering assumptions
Testing strategy
```

Root Court 中不得出现未注释的 `unsafe` 块。

### 7.2 C

C 是受限语言，不是默认语言。C 的主要用途：

```text
UEFI/firmware ABI shim
C ABI exported headers
早期 boot compatibility glue
现有硬件表解析库的迁移期封装
外部驱动兼容层
极少量需要与 C 工具链强绑定的代码
```

C 代码约束：

```text
不得直接持有 Root Court 全局可变状态
不得直接修改 capability registry
不得直接修改 namespace
不得直接执行未经 Rust 封装的资源授权逻辑
不得调用 libc
不得使用隐式动态内存分配
必须通过显式 FFI 边界进入 Rust 核心逻辑
```

C 只可以作为“边界适配层”，不得成为 Root Court 的策略主体。

### 7.3 x86-64 Assembly

汇编只用于 Rust/C 无法正确或合理表达的最低层区域。

允许汇编区域：

```text
UEFI handoff / early entry
long mode transition residual path
AP startup trampoline
GDT/IDT/TSS early setup helper
interrupt entry / exit stubs
context switch primitive
syscall/sysret 或 sysenter/sysexit 入口
VMX root/non-root transition stubs
VMCALL/VMLAUNCH/VMRESUME wrapper
TSC / MSR / control register access thin wrappers
TLB shootdown helper
```

汇编代码约束：

```text
必须最小化
必须有 Rust 或 C 类型化封装
不得包含策略逻辑
不得隐式修改 Root Court 对象状态
必须保存/恢复 ABI 指定寄存器
必须明确中断状态约定
必须经过单元测试或仿真测试覆盖关键路径
```

### 7.4 推荐仓库结构

```text
court-kernel/
  rfc/
    RFC-0001.md
  root-court/
    Cargo.toml
    src/
      lib.rs
      cap/
      namespace/
      corridor/
      placement/
      loader/
      memory/
      irq/
      device/
      trace/
      health/
      arch/x86_64/
    asm/x86_64/
    cshim/
    linker/
  court-api/
    rust/
    c/
    idl/
  courtlet-sdk/
    rust/
    c/
  courts/
    net/
    storage/
    crypto/
    app/
    name/
    policy/
  tools/
    manifestc/
    capviz/
    traceview/
    qemu-runner/
  tests/
    unit/
    integration/
    qemu/
    fuzz/
```

---

## 8. 对象模型

### 8.1 Court 对象

```rust
struct Court {
    id: CourtId,
    name: Name,
    kind: CourtKind,
    state: CourtState,
    images: ImageSet,
    placement: PlacementPolicy,
    cspace: CapabilitySpace,
    namespace: NamespaceView,
    corridors: CorridorSet,
    health: HealthPolicy,
    trace: TracePolicy,
}
```

Court 状态机：

```text
Declared -> Loaded -> Isolated -> Linked -> Running -> Suspended
Running -> Faulted -> Quarantined -> Restarting -> Running
Running -> Terminating -> Revoked -> Destroyed
```

### 8.2 Courtlet 对象

```rust
struct Courtlet {
    id: CourtletId,
    court: CourtId,
    vcpu: Option<VcpuId>,
    host_cpu: Option<CpuId>,
    address_space: AddressSpaceId,
    entry: VirtAddr,
    state: CourtletState,
    time_budget: TimeBudgetId,
}
```

### 8.3 Capability 对象

Capability 是不可伪造授权凭证。Root Court 是 capability 的根发行者。

```rust
struct Capability {
    id: CapId,
    object: ObjectId,
    object_type: ObjectType,
    rights: Rights,
    generation: Generation,
    parent: Option<CapId>,
    revocation_domain: RevocationDomainId,
    delegable: bool,
    attenuable: bool,
    expires_at: Option<TimePoint>,
}
```

### 8.4 Rights

```text
read
write
execute
map
unmap
send
recv
mint
delegate
attenuate
revoke
observe
configure
bind
schedule
signal
reset
own
```

### 8.5 能力类型

```text
MemoryRegionCap
AddressSpaceCap
CpuSetCap
TimeBudgetCap
IrqCap
DeviceCap
DeviceQueueCap
ChannelCap
SharedRingCap
BulkMapCap
SignalCap
NameLookupCap
NameBindCap
TraceCap
PolicyCap
CourtImageCap
CourtControlCap
```

### 8.6 能力派生

能力派生必须满足单调削弱原则：

```text
child.rights ⊆ parent.rights
child.object == parent.object 或 child.object 是 parent.object 的子对象
child.revocation_domain 可等于或细化 parent.revocation_domain
```

不得通过 delegation 或 attenuation 扩大权限。

### 8.7 能力撤销

Capability revocation 必须影响所有依赖该 capability 的活跃访问路径，包括：

```text
namespace open handle
EPT mapping
IOMMU mapping
IRQ routing
MSI-X vector assignment
corridor endpoint
shared ring access
device queue ownership
trace subscription
```

撤销必须支持两种语义：

```text
Best-effort revocation：用于非安全关键资源，允许延迟回收。
Synchronous revocation：用于安全关键资源，返回前必须使后续访问失败。
```

---

## 9. 命名空间模型

### 9.1 原则

命名空间用于发现，不用于直接授权。

```text
lookup(name) -> descriptor
open(descriptor, requested_rights, capability) -> handle/capability
```

路径名不得赋予权限。任何实际操作都必须经 capability 检查。

### 9.2 命名空间结构

推荐初始结构：

```text
/sys/cpu/topology
/sys/cpu/hfi
/sys/mm/pools
/sys/pci/devices
/sys/policy/placement

/court/root
/court/name0
/court/net0
/court/net0/if/eth0
/court/net0/queue/rx0
/court/net0/corridor/packet-rx
/court/storage0/nvme/ns0
/court/crypto0/sign/ed25519

/cap/local
/trace/court/net0/latency
/trace/corridor/packet-rx
/policy/qos/net0
```

### 9.3 Namespace View

每个 Court 拥有自己的 namespace view。Root Court 可以为 Court 装配不同 view。

```text
global namespace graph
        ↓ projection
court-local namespace view
        ↓ lookup
object descriptor
        ↓ capability-mediated open
handle/capability
```

### 9.4 Mount / Bind

Namespace bind 操作必须持有 NameBindCap。没有 NameBindCap 的 Court 不得向 namespace 注册服务。

示例：

```text
NameBindCap(/court/net0/corridor)
+ CorridorCap(packet-rx, observe|recv)
=> bind /court/net0/corridor/packet-rx
```

---

## 10. 连廊模型

Corridor 是跨庭接口的基本对象。每个 corridor 都必须具备：

```text
唯一 ID
两端 CourtId
协议 ID
传输类型
所需 capability
QoS 描述
trace policy
revocation policy
health policy
```

```rust
struct Corridor {
    id: CorridorId,
    from: CourtId,
    to: CourtId,
    protocol: ProtocolId,
    transport: TransportKind,
    required_caps: CapabilitySet,
    qos: QosPolicy,
    trace: TracePolicy,
    revocation: RevocationPolicy,
    health: HealthPolicy,
}
```

### 10.1 标准传输类型

RFC-0001 定义五种 corridor transport。

#### 10.1.1 Control Channel

用途：低频控制消息、配置、状态查询、typed RPC。

约束：

```text
消息必须有 type id
消息必须有 bounded size
默认最大 4 KiB
必须支持 request/response 与 one-way event
必须支持 timeout
必须可 trace
```

#### 10.1.2 Shared Ring

用途：网络包、日志、块 I/O、小对象批处理。

约束：

```text
ring memory 必须由 SharedRingCap 授权
producer/consumer index 必须使用原子操作
必须定义 backpressure
必须定义 wraparound 行为
必须定义 revoke 后行为
必须支持 trace sampling
```

x86 上虽然 TSO 简化一部分内存排序，但实现必须使用语言层面的 Acquire/Release 原子语义，并对 MMIO doorbell 使用显式 fence 或架构封装。

#### 10.1.3 Bulk Mapping

用途：大对象共享、buffer loan、zero-copy bulk transfer。

约束：

```text
必须持有 MemoryRegionCap
必须定义 read-only / read-write / copy-on-grant
必须定义 lifetime
必须支持撤销策略
```

#### 10.1.4 Device Queue

用途：NIC/NVMe/GPU 等设备队列直通或半直通。

约束：

```text
必须持有 DeviceQueueCap
必须经过 IOMMU domain 授权
必须绑定 IRQ/MSI-X capability
必须定义 ownership
必须支持 crash reclaim
```

#### 10.1.5 Signal / Doorbell

用途：事件通知、中断转发、定时器、watchdog、轻量唤醒。

约束：

```text
必须持有 SignalCap
必须支持 edge/level 语义声明
必须支持 rate limit
必须可 trace
```

### 10.2 Corridor IDL

跨庭协议必须有 IDL。IDL 至少描述：

```text
protocol id
version
message types
object capabilities required
transport kind
error model
timeout model
revocation behavior
trace fields
compatibility policy
```

示例：

```yaml
protocol: court.net.packet.v1
transport: shared_ring
requires:
  - SharedRingCap: recv|observe
messages:
  - PacketDesc:
      fields:
        - offset: u32
        - len: u16
        - flags: u16
        - flow_id: u64
errors:
  - CAP_REVOKED
  - RING_FULL
  - PEER_DOWN
```

---

## 11. 启动流程

### 11.1 Stage 0：固件与装载

```text
UEFI -> bootloader -> Root Court image + manifest bundle
```

Manifest bundle 包含：

```text
root-court.elf
court-net.elf
court-storage.elf
court-crypto.elf
court-app.elf
namespace.init.yaml
capability.init.yaml
placement.policy.yaml
device.policy.yaml
```

### 11.2 Stage 1：Root Court early init

CPU0 完成 early entry 后：

```text
1. 建立临时栈
2. 建立 early GDT/IDT
3. 进入 long mode 环境确认
4. 建立 early page table
5. 解析 UEFI memory map
6. 初始化 early allocator
7. 初始化日志输出
8. 解析 ACPI / MADT / SRAT / DMAR
9. 枚举 CPUID topology
10. 初始化 APIC/x2APIC
11. 初始化 Root CSpace
12. 初始化 Root Namespace
```

### 11.3 Stage 2：多核心启动

```text
1. 发送 INIT/SIPI 启动 AP
2. AP 进入 trampoline
3. AP 切换到 long mode runtime
4. AP 注册 CpuId
5. Root Court 建立 CpuSetCap
6. 读取拓扑、cache、NUMA、core class 信息
7. 建立 TimeBudget root object
```

### 11.4 Stage 3：虚拟化与 IOMMU 初始化

```text
1. 检查 VMX/SVM 支持
2. 启用 VMX root 或 SVM host mode
3. 初始化 EPT/NPT root templates
4. 初始化 IOMMU root table
5. 建立 DMA remapping domains
6. 建立 interrupt remapping policy
```

如果 VMX/SVM 或 IOMMU 缺失，Root Court 必须标记安全级别降级。

### 11.5 Stage 4：Court Image 加载

```text
1. 解析 Court manifest
2. 验证 Court image
3. 分配 Court address space
4. 建立 EPT/NPT mappings
5. 注入 bootstrap cap
6. 建立初始 namespace view
7. 建立初始 corridors
8. 创建 Courtlet VCPU
9. 启动 Courtlet
```

### 11.6 Stage 5：应用与服务启动

```text
1. App Court 启动
2. App Court lookup /court/net0
3. App Court 请求 open corridor
4. Policy Court 或 Root Court 检查 capability
5. Root Court 发放 attenuated corridor cap
6. App Court 使用 corridor 与 Net Court 通信
```

---

## 12. 调度与放置

Court Kernel 采用两层调度：

```text
Root Court：庭级放置、核心分配、时间预算、隔离约束、热/功耗策略
Court 内部：线程、协程、事件循环、批处理、局部优先级
```

Root Court 不得管理每个庭内部所有细粒度任务，除非该庭主动将内部任务暴露为可调度对象。

### 12.1 放置输入

```text
Court kind
Court manifest
requested core class
min/max cores
exclusive requirement
security isolation level
device affinity
NUMA locality
cache sharing risk
HFI performance hint
HFI efficiency hint
thermal state
QoS deadline
historical trace data
```

### 12.2 放置策略

示例伪代码：

```rust
fn place_court(court: &CourtManifest, hw: &HardwareState) -> Placement {
    let candidates = hw.cores()
        .filter(|c| c.satisfies_security(court.security))
        .filter(|c| c.satisfies_features(court.features))
        .filter(|c| c.near_required_devices(court.device_affinity));

    candidates.max_by_score(|c| {
        court.weights.perf * hw.hfi_perf(c)
      + court.weights.eff  * hw.hfi_eff(c)
      - court.weights.thermal * hw.thermal_pressure(c)
      - court.weights.cache_risk * hw.cache_sharing_risk(c, court)
      - court.weights.migration * migration_cost(court, c)
    })
}
```

### 12.3 TimeBudgetCap

CPU 时间必须 capability 化。

```rust
struct TimeBudgetCap {
    budget_ns: u64,
    period_ns: u64,
    deadline_ns: Option<u64>,
    criticality: Criticality,
    transferable: bool,
}
```

一个 Courtlet 没有 TimeBudgetCap 不得运行。

### 12.4 安全庭放置

安全敏感庭 SHOULD 使用：

```text
独占物理核心
禁用 SMT sibling 共驻
限制共享 cache 干扰
必要时使用 CAT/RDT 或 page coloring
固定 TimeBudget
最小 namespace view
最小 corridor
constant-time crypto implementation
```

不得声称 E-core 天然提供侧信道安全。

---

## 13. 内存模型

### 13.1 根所有权

Root Court 初始拥有全部物理内存。任何 Court 对内存的访问都必须通过 MemoryRegionCap、AddressSpaceCap 或 SharedRingCap 表达。

### 13.2 内存对象

```rust
struct MemoryRegion {
    id: MemoryRegionId,
    phys_start: PhysAddr,
    len: usize,
    attributes: MemoryAttributes,
    owner: CourtId,
    sharing: SharingPolicy,
    generation: Generation,
}
```

### 13.3 Mapping

```text
Courtlet virtual address -> EPT/NPT -> physical memory
```

Root Court 必须保证：

```text
没有 MemoryRegionCap 不得建立 mapping
没有 map right 不得映射
没有 write right 不得建立 writable mapping
没有 execute right 不得建立 executable mapping
revocation 后映射必须失效
```

### 13.4 Shared Memory

共享内存不得隐式创建。必须通过 SharedRegionCap 或 SharedRingCap 显式建立。

共享内存必须记录：

```text
participants
rights per participant
cacheability
memory ordering semantics
revocation behavior
trace policy
```

---

## 14. 设备与中断模型

### 14.1 设备所有权

设备访问必须 capability 化。

设备可按三层授权：

```text
Level 1: full device assignment
Level 2: virtual function / queue assignment
Level 3: brokered device service
```

### 14.2 DeviceCap

```rust
struct DeviceCap {
    device_id: DeviceId,
    rights: DeviceRights,
    iommu_domain: IommuDomainId,
    mmio_regions: Vec<MemoryRegionId>,
    irq_caps: Vec<IrqCapId>,
}
```

### 14.3 DeviceQueueCap

```rust
struct DeviceQueueCap {
    device_id: DeviceId,
    queue_id: QueueId,
    rights: QueueRights,
    dma_regions: Vec<MemoryRegionId>,
    irq: Option<IrqCapId>,
}
```

### 14.4 IRQ / MSI-X

中断绑定必须持有 IrqCap。

```text
IRQ vector -> Root Court routing table -> Courtlet event injection / Signal corridor
```

Courtlet 不得未经授权直接重编程全局中断路由。

### 14.5 DMA 隔离

任何可发起 DMA 的设备队列都必须绑定 IOMMU domain。没有 IOMMU 的平台只能运行在弱隔离模式，并必须在系统状态中显式标记。

---

## 15. Court Manifest

每个庭必须有 manifest。Manifest 是 Root Court 加载与治理 Court 的声明式输入。

示例：

```yaml
court: net.rx
version: 0.1
kind: network_dataplane
image:
  arch: x86_64
  path: court-net-rx.elf
language:
  primary: rust
  allowed_ffi:
    - c
placement:
  preferred_core_class: efficient
  allow_fallback: performant
  min_cores: 1
  max_cores: 4
  numa_affinity: near_device
  exclusive: false
security:
  isolation: vmx_ept
  allow_smt: true
qos:
  latency_us_p99: 80
  bandwidth_mbps: 10000
caps:
  request:
    - device:nic0.queue.rx[0..3]:read|write
    - memory:packet_pool:read|write|map
    - irq:msix.nic0.rx[0..3]:bind
corridors:
  provides:
    - name: /court/net0/packet/rx
      protocol: court.net.packet.v1
      transport: shared_ring
      rights: recv|observe
trace:
  expose:
    - /trace/court/net0/latency
    - /trace/court/net0/rx_drops
health:
  restart: on_fault
  max_restarts_per_minute: 3
```

Manifest 中声明的权限只是请求，不等于授予。Root Court 或 Policy Court 必须根据系统策略授予 capability。

---

## 16. ABI 与调用模型

### 16.1 Root Court Invocation

Courtlet 与 Root Court 的交互通过 typed invocation 完成。

抽象 ABI：

```c
typedef uint64_t ck_cap_t;
typedef uint64_t ck_op_t;
typedef uint64_t ck_status_t;

struct ck_msg {
    uint64_t words[6];
};

ck_status_t ck_cap_invoke(
    ck_cap_t target,
    ck_op_t op,
    const struct ck_msg* in,
    struct ck_msg* out
);
```

在 VMX non-root 模式下，`ck_cap_invoke` 可以由 `VMCALL` 承载。在 hosted prototype 中，可以由普通 syscall、ioctl、Unix socket 或 shared memory 模拟。

### 16.2 错误码

基础错误码：

```text
CK_OK
CK_ERR_BAD_CAP
CK_ERR_NO_RIGHT
CK_ERR_REVOKED
CK_ERR_NOT_FOUND
CK_ERR_INVALID_OBJECT
CK_ERR_INVALID_STATE
CK_ERR_TIMEOUT
CK_ERR_PEER_DOWN
CK_ERR_QUOTA
CK_ERR_UNSUPPORTED
CK_ERR_FAULT
```

### 16.3 Capability Invocation

所有 capability invocation 必须满足：

```text
检查 cap 是否存在
检查 generation 是否匹配
检查 object type 是否匹配
检查 rights 是否满足 op
检查 object state 是否允许 op
执行 op
记录 trace/audit
```

---

## 17. 安全模型

### 17.1 威胁模型

RFC-0001 假设可能存在：

```text
恶意 App Court
被攻破的普通功能 Court
有 bug 的驱动 Court
试图越权访问内存的 Courtlet
试图越权访问设备 MMIO 的 Courtlet
试图伪造 capability 的 Courtlet
试图滥用 corridor 造成 DoS 的 Courtlet
错误或恶意的 manifest
```

RFC-0001 暂不覆盖：

```text
物理攻击者
恶意固件
恶意 CPU microcode
Root Court 自身被完全攻破
所有微架构侧信道的完全消除
```

### 17.2 安全不变量

实现必须维护以下不变量：

1. 没有 capability 就没有权限。
2. namespace lookup 不得直接授予权限。
3. Courtlet 不得访问 EPT/NPT 未映射内存。
4. Courtlet 不得访问未授权 MMIO。
5. 可 DMA 设备不得访问 IOMMU 未授权内存。
6. IRQ/MSI-X 路由必须由 IrqCap 授权。
7. TimeBudgetCap 是 Courtlet 获得 CPU 时间的必要条件。
8. Capability delegation 不得扩大权限。
9. Capability revocation 必须使后续访问失败。
10. Root Court 不得信任 Court Image 内容。
11. Court crash 不得直接破坏其他 Court 的地址空间。
12. Corridor 必须可观测、可撤销、可限流。

### 17.3 侧信道立场

Court Kernel 不声称通过庭划分自动解决侧信道。安全庭需要组合使用：

```text
独占核心
禁用 SMT sibling
cache isolation hint
page coloring / CAT if available
constant-time implementation
bounded corridor
noise-aware trace
```

---

## 18. 观测、治理与健康管理

### 18.1 Trace

每个 corridor、Court、capability operation SHOULD 暴露 trace endpoint。

示例：

```text
/trace/court/net0/latency
/trace/court/net0/faults
/trace/corridor/packet-rx/throughput
/trace/cap/revocations
/trace/placement/decisions
```

Trace 访问必须由 TraceCap 控制。观测权不等于控制权。

### 18.2 Health Monitor

Root Court 必须支持：

```text
Court heartbeat
Courtlet fault detection
corridor peer-down detection
restart policy
quarantine policy
cap revocation on fault
crash log collection
```

### 18.3 Policy Court

复杂策略 MAY 从 Root Court 迁移到 Policy Court。Policy Court 可以给 Root Court 提供决策建议，但 Root Court 必须保留最终资源安全检查。

---

## 19. 第一版核心庭划分

### 19.1 Root Court

负责资源、能力、命名根、隔离、加载、放置、连廊、trace 和 health。

### 19.2 Name Court

可选。早期命名可留在 Root Court；后续迁出为 Name Court。

职责：

```text
namespace graph
namespace view projection
mount/bind/union
watch/observe
```

### 19.3 Net Court

第一版样板庭。

职责：

```text
NIC queue management
packet RX/TX
shared ring corridor
basic routing 或 packet dispatch
trace packet latency/drop
```

### 19.4 Storage Court

职责：

```text
block device queue
NVMe/AHCI/virtio-blk prototype
block cache
filesystem service, optional
```

### 19.5 Crypto Court

职责：

```text
key custody
sign/verify
authenticated random service
attestation, future
```

默认策略：

```text
独占核心
最小 namespace
最小 corridor
严格 trace 权限
```

### 19.6 App Court

职责：

```text
tiny runtime
WASI-like runtime, optional
POSIX subset, future
Linux syscall translation, non-goal for RFC-0001
```

---

## 20. MVP 路线

### 20.1 MVP-0：Hosted Prototype

在 Linux/KVM/QEMU 上模拟庭内核对象模型。

目标：

```text
process/container = Court
mmap/shared memory = Corridor
Unix socket = Control Channel
cgroup/cpuset = weak TimeBudget/CpuSet simulation
FUSE/9P-like prototype = Namespace simulation
```

验收：

```text
App Court 能 lookup /court/net0/packet/rx
没有 cap 无法 open
获得 cap 后可以通信
撤销 cap 后通信失败
trace 可见 corridor latency
```

### 20.2 MVP-1：裸机 Root Court

目标：

```text
UEFI boot
long mode
basic paging
physical allocator
GDT/IDT
AP startup
x2APIC timer
IPI ping-pong
Root CSpace
Root Namespace
```

### 20.3 MVP-2：Trusted Courtlet

目标：

```text
加载两个 Court Image
建立独立页表或隔离域雏形
建立 control channel
建立 shared ring
cap-mediated open/revoke
```

说明：此阶段不具备最终安全性。

### 20.4 MVP-3：VMX/EPT Courtlet

目标：

```text
Root Court 进入 VMX root
Courtlet 运行 VMX non-root
EPT 限制内存
VMCALL 承载 Root invocation
EPT violation 可捕获
cap revoke 可失效映射
```

### 20.5 MVP-4：Net Court Demo

目标：

```text
virtio-net 或真实 NIC
Net Court 拥有 RX/TX queue
App Court 通过 Shared Ring 发包收包
Net Court crash 后 Root Court 重启
```

### 20.6 MVP-5：异构调度 Demo

目标：

```text
检测 P/E core 或 classic/dense core
根据 manifest 放置 Court
展示 perf/watt 或 p99 latency 改善
展示 thermal pressure 下重新放置
```

---

## 21. 测试与评估

### 21.1 架构指标

```text
Root Court LOC
unsafe LOC
C LOC
Assembly LOC
capability object count
namespace lookup latency
cap mint/delegate/revoke latency
corridor establish latency
```

### 21.2 性能指标

```text
control channel ping-pong latency
shared ring throughput
cross-court RPC p50/p99
VM exit cost
EPT fault handling cost
packet RX/TX throughput
storage IOPS
```

### 21.3 隔离指标

```text
非法内存访问是否被阻断
非法 MMIO 是否被阻断
非法 DMA 是否被阻断
cap revoke 后访问是否失败
Court crash 是否影响其他 Court
```

### 21.4 异构指标

```text
P-core/E-core occupancy
performance per watt
p99 latency under thermal pressure
migration count
QoS violation count
```

### 21.5 安全测试

```text
capability fuzzing
manifest fuzzing
namespace fuzzing
corridor protocol fuzzing
EPT violation tests
IOMMU violation tests
VMCALL ABI fuzzing
unsafe boundary audit
```

---

## 22. 构建、验证与代码规范

### 22.1 Rust 规范

```text
#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)] for public APIs, after MVP-1
#![forbid(unused_must_use)]
```

Root Court 公共 API 必须显式表达错误，不得 panic。

### 22.2 C 规范

```text
-fno-builtin
-ffreestanding
-fno-stack-protector, unless runtime support exists
-no libc
-no hidden malloc/free
```

C ABI 头文件必须从单一 IDL 或 Rust/C 共享定义生成，避免手动漂移。

### 22.3 Assembly 规范

```text
每个符号必须有 ABI 注释
每个入口必须说明栈布局
每个入口必须说明中断状态
每个入口必须说明保存/破坏寄存器
```

### 22.4 Unsafe 审计

每个 unsafe 模块维护一个 `SAFETY.md`：

```text
Unsafe operation list
Invariant list
Caller obligations
Tests
Known risks
Review history
```

---

## 23. 主要风险

### 23.1 Root Court 膨胀

风险：Root Court 吸收服务后退化为宏内核。

缓解：Root Court 只保留资源、安全和治理根逻辑。普通服务必须庭化。

### 23.2 Namespace 瓶颈

风险：统一命名空间成为全局锁。

缓解：每庭 namespace view、snapshot、cap caching、hot path 不 lookup。

### 23.3 Corridor 协议碎片化

风险：每个庭定义私有协议导致系统不可治理。

缓解：标准化 transport，要求 IDL 和 trace schema。

### 23.4 VMX/EPT 成本

风险：VM exit 和 EPT 操作导致开销过高。

缓解：hot path 使用 shared ring、device queue、batching；control path 才频繁进入 Root Court。

### 23.5 设备隔离粒度不足

风险：某些设备无法单队列隔离。

缓解：使用 brokered device service；无法安全隔离时不得直通。

### 23.6 侧信道过度承诺

风险：把物理核心隔离误认为完整侧信道解决方案。

缓解：RFC 明确不承诺完全侧信道安全；安全庭采用组合缓解。

---

## 24. 开放问题

1. Capability revocation 是否采用 epoch-based、reference-counted 还是 capability derivation tree walk？
2. Namespace view 是否由 Root Court 直接维护，还是尽早迁移到 Name Court？
3. Courtlet 的默认 ABI 使用 VMCALL、SYSCALL 还是 architecture-neutral shim？
4. Shared Ring 是否采用统一 descriptor，还是允许协议自定义 descriptor？
5. TimeBudgetCap 是否需要支持硬实时 deadline？
6. Policy Court 的建议如何被 Root Court 验证？
7. 如何定义 Court Image 的签名、版本和可回滚策略？
8. 是否为 Rust Courtlet SDK 提供 async runtime？
9. 第一个真实设备 demo 应选择 virtio-net、e1000、ixgbe、NVMe 还是 virtio-blk？
10. 如何逐步引入形式化验证？先验证 cap graph、状态机，还是 Root Court 子集？

---

## 25. RFC-0001 的最低合规实现

一个实现若要被称为符合 RFC-0001，至少必须满足：

```text
1. 存在 Root Court，且 Root Court 是唯一最高可信根。
2. 存在 capability object model。
3. namespace lookup 不直接授予权限。
4. 至少支持两个 Court。
5. 至少支持一个 Control Channel corridor。
6. 至少支持一个 Shared Ring corridor。
7. 支持 capability mint/delegate/revoke。
8. 支持 Court manifest。
9. 支持 trace endpoint。
10. 支持 Court crash containment 的最小机制。
11. 明确标记是否处于 trusted bring-up 或 VMX/EPT isolated 模式。
12. Rust 是默认核心实现语言，C/Assembly 受限使用。
```

---

## 26. 参考文献

[R1] Pike et al., “Plan 9 from Bell Labs”, Computing Systems, 1995.  
[R2] Engler et al., “Exokernel: An Operating System Architecture for Application-Level Resource Management”, SOSP 1995.  
[R3] Baumann et al., “The Multikernel: A new OS architecture for scalable multicore systems”, SOSP 2009.  
[R4] Klein et al., “seL4: Formal Verification of an OS Kernel”, SOSP 2009.  
[R5] seL4 Documentation, Capability System and CSpace Tutorials.  
[R6] Fuchsia Documentation, Zircon Handles and Rights.  
[R7] Peter et al., “Arrakis: The Operating System is the Control Plane”, OSDI 2014.  
[R8] Belay et al., “IX: A Protected Dataplane Operating System for High Throughput and Low Latency”, OSDI 2014.  
[R9] Belay et al., “Dune: Safe User-level Access to Privileged CPU Features”, OSDI 2012.  
[R10] Intel, “Intel 64 and IA-32 Architectures Software Developer’s Manual”.  
[R11] Intel, “Intel Virtualization Technology for Directed I/O”.  
[R12] Rust Embedded Working Group, “The Embedded Rust Book: no_std”.  
[R13] Rust Project, “The Rustonomicon”.  
[R14] Linux Kernel Documentation, Rust for Linux and no_std constraints.  

---

## 27. 附录 A：最小 corridor ring descriptor

```rust
#[repr(C)]
pub struct CkRingHeader {
    pub magic: u32,
    pub version: u16,
    pub flags: u16,
    pub capacity: u32,
    pub desc_size: u16,
    pub _reserved: u16,
    pub producer: AtomicU64,
    pub consumer: AtomicU64,
}

#[repr(C)]
pub struct CkPacketDesc {
    pub offset: u32,
    pub len: u16,
    pub flags: u16,
    pub flow_id: u64,
}
```

约束：

```text
producer 仅由 producer endpoint 更新
consumer 仅由 consumer endpoint 更新
descriptor 写入完成后 producer 使用 Release 更新
consumer 读取 producer 使用 Acquire
MMIO doorbell 必须经过 arch fence wrapper
```

---

## 28. 附录 B：Root Court unsafe boundary 示例

```rust
pub mod arch {
    pub mod x86_64 {
        pub mod vmx;
        pub mod ept;
        pub mod apic;
        pub mod msr;
        pub mod cr;
        pub mod interrupt;
    }
}
```

每个模块必须包含：

```text
SAFETY.md
unit tests where possible
QEMU integration tests
fault injection tests
```

---

## 29. 附录 C：第一阶段 demo 场景

```text
Root Court
  ├── Name subsystem
  ├── Policy subsystem
  ├── Net Court
  ├── Crypto Court
  └── App Court
```

演示流程：

```text
1. App Court lookup /court/net0/packet/rx
2. App Court 无 cap，open 失败
3. Policy 授予 attenuated SharedRingCap
4. App Court open 成功
5. App Court 通过 shared ring 发送 packet descriptor
6. Net Court 消费 descriptor
7. Root Court trace corridor latency
8. Root Court revoke cap
9. App Court 后续 send 失败
10. Net Court crash
11. Root Court quarantine + restart Net Court
12. App Court 不崩溃，仅观察到 PEER_DOWN
```

---

## 30. 一句话定义

Court Kernel 是一种功能切片、异构感知、capability 管控、统一命名、显式连廊的 OS 架构；它不试图成为更好的 Linux，而是试图从 2026 年的 x86 异构多核现实出发，重新组织操作系统的基本结构。
