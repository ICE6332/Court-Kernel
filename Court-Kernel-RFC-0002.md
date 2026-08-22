# Court Kernel RFC-0002

**标题**：Court Kernel ABI 与 Corridor ABI 草案  
**状态**：Draft 0.1  
**日期**：2026-04-27  
**语言**：中文  
**继承文档**：Court Kernel RFC-0001  
**目标平台**：经典现代 x86-64；优先覆盖 VMX/EPT、SVM/NPT、IOMMU、APIC/x2APIC 平台  
**实现语言**：Rust + C + x86-64 Assembly  
**适用范围**：Root Court 与 Courtlet 之间的系统调用/超调用 ABI、跨庭 Corridor 传输 ABI、IDL 与版本兼容规则  

---

## 0. 摘要

RFC-0001 定义了 Court Kernel 的总体架构：Root Court 作为最小可信根，Court/Courtlet 作为功能庭，Capability 作为权限根，Namespace 作为发现与治理面，Corridor 作为跨庭接口。

RFC-0002 在 RFC-0001 的基础上定义 **Court Kernel ABI** 与 **Corridor ABI**：

1. Courtlet 如何进入运行。
2. Courtlet 如何通过统一调用入口请求 Root Court 服务。
3. Capability 如何以 ABI 对象形式被传递、调用、衰减、撤销。
4. Namespace lookup/open/bind/watch 如何编码。
5. Corridor 如何创建、映射、发送、接收、通知、观测、撤销。
6. Shared Ring、Control Channel、Bulk Mapping、Device Queue、Signal/Doorbell 五类 Corridor 的最低 ABI。
7. Rust、C、x86-64 Assembly 在 ABI 边界上的职责与约束。
8. ABI 版本、结构体布局、错误码、操作码、测试向量和最低合规实现。

本 RFC 的核心定义是：

> Court Kernel ABI 是 Courtlet 与 Root Court 之间的 capability invocation ABI；Corridor ABI 是 Court 与 Court 之间通过 typed transport 进行协作的稳定契约。

本 RFC 的重要设计选择：

1. **单一 Root Invocation 入口**：所有 Courtlet 到 Root Court 的操作通过 `ck_cap_invoke` 完成。
2. **路径名不参与权限判断**：Namespace ABI 只返回可打开对象描述或 capability，实际授权由 capability rights 决定。
3. **跨庭数据面优先走 Corridor**：小控制消息走 Channel，大数据走 Shared Ring，超大对象走 Bulk Mapping，设备快路径走 Device Queue，通知走 Signal/Doorbell。
4. **实验路径与最终路径共享 ABI**：trusted bring-up 阶段可以用函数调用或 `syscall` 模拟；VMX/EPT 阶段使用 `vmcall`/`vmmcall` 进入 Root Court，但语义保持一致。
5. **ABI 结构必须可由 C 与 Rust 同时稳定表达**：所有 ABI 结构使用固定宽度整数、`repr(C)`、8 字节对齐、显式 `size`/`version`/`flags` 字段，不跨 ABI 暴露 Rust enum、trait、slice、Vec、String 或 host pointer。

---

## 1. 术语与规范性语言

本文使用 RFC 风格的规范性措辞：

- **MUST / 必须**：实现必须满足，否则不符合 RFC-0002。
- **SHOULD / 应该**：强烈建议满足；不满足时必须给出工程理由。
- **MAY / 可以**：允许实现选择。
- **MUST NOT / 不得**：实现不得采用。

### 1.1 继承自 RFC-0001 的术语

| 术语 | 定义 |
|---|---|
| Court / 庭 | 一个功能垂直切片，是庭内核的基本组织单元。 |
| Courtlet / 庭核或庭运行时 | 某个庭在具体核心或执行域中运行的最小内核/运行时镜像。 |
| Root Court / 枢庭 | 唯一最高可信根，负责硬件资源、隔离、能力、命名、加载、连廊和放置。 |
| Capability / 能力 | 不可伪造的授权凭证，表示对某对象的一组操作权限。 |
| Namespace / 命名空间 | 用于发现资源、服务、连廊、能力入口和观测点的命名系统。 |
| Corridor / 连廊 | 庭与庭之间的显式通信接口与传输对象。 |

### 1.2 RFC-0002 新增术语

| 术语 | 定义 |
|---|---|
| CK ABI | Court Kernel ABI，即 Courtlet 与 Root Court 之间的调用、对象、错误码和版本规则。 |
| Corridor ABI | 跨庭传输对象的布局、状态机、权限、通知和撤销规则。 |
| Invocation | 对 capability 指向对象的一次 typed operation 调用。 |
| Call Gate | Courtlet 进入 Root Court 的实际机器入口，可以是 `vmcall`、`vmmcall`、`syscall` 或 bring-up 直接调用。 |
| ABI Message | `ck_msg` 描述的一次输入/输出缓冲区与 capability 参数集合。 |
| Local Handle | Courtlet 本地 capability 表中的 64-bit capability handle。 |
| Protocol ID | Corridor 上层协议的 128-bit 标识。 |
| Endpoint | Corridor 的一端，通常绑定到某个 Court 或 Courtlet。 |
| Doorbell | 用于通知另一端的轻量 signal 或 IRQ-like event。 |

---

## 2. 与 RFC-0001 的关系

RFC-0001 定义“系统是什么”，RFC-0002 定义“系统边界怎么说话”。

RFC-0002 不改变 RFC-0001 的总体架构，而是把以下 RFC-0001 对象落成 ABI：

```text
Court              -> ck_court_desc / ck_court_status
Courtlet           -> ck_boot_info / ck_courtlet_entry
Capability         -> ck_cap_t / ck_rights_t / ck_cap_invoke
Namespace          -> CK_OP_NS_LOOKUP / CK_OP_NS_OPEN / CK_OP_NS_BIND
Corridor           -> ck_corridor_desc / ck_channel_msg / ck_ring_header
MemoryRegion       -> CK_OBJ_MEMORY / CK_OP_MEM_MAP / CK_OP_MEM_SHARE
DeviceQueue        -> CK_OBJ_DEVICE_QUEUE / ck_device_queue_desc
Signal             -> CK_OBJ_SIGNAL / CK_OP_SIGNAL_RAISE / CK_OP_SIGNAL_WAIT
TracePoint         -> CK_OBJ_TRACE / CK_OP_TRACE_EMIT / CK_OP_TRACE_SUBSCRIBE
```

RFC-0002 尤其服务于当前 0001 实验阶段。实验实现可以按以下顺序逐步采用：

```text
Stage A: Rust/C 内部函数调用模拟 ck_cap_invoke
Stage B: QEMU trusted bring-up 使用 syscall 或直接 trap
Stage C: VMX non-root Courtlet 使用 vmcall 进入 Root Court
Stage D: IOMMU + Device Queue + Shared Ring 数据面
Stage E: ABI compliance test + fuzz + revocation test
```

---

## 3. 设计目标

### 3.1 ABI 目标

1. **单一调用入口**：Courtlet 对 Root Court 的所有请求 SHOULD 通过 `ck_cap_invoke` 表达。
2. **强类型对象调用**：每次调用 MUST 指定目标 capability 与 operation ID。
3. **权限可验证**：Root Court MUST 在执行 operation 前验证 capability 类型、rights、generation、revocation state。
4. **跨语言稳定**：ABI MUST 可由 Rust、C、Assembly 共同实现。
5. **跨隔离模式稳定**：同一 ABI MUST 可运行在 trusted bring-up、VMX/EPT、SVM/NPT 模式下。
6. **热路径可优化**：ABI MUST 支持 fast path，例如 signal、ring notification、yield、trace emit。
7. **演化友好**：ABI 结构 MUST 包含 `size`、`version`、`flags` 或保留字段，支持向后兼容。
8. **拒绝隐式共享状态**：跨庭状态共享 MUST 通过 MemoryRegionCap、SharedRingCap、DeviceQueueCap 等显式 capability 建立。

### 3.2 Corridor 目标

1. **连廊显式化**：每个跨庭接口 MUST 是可命名、可授权、可观测、可撤销的对象。
2. **控制面与数据面分离**：控制消息、数据流、设备队列、信号通知不得混为单一 IPC。
3. **零拷贝友好**：Shared Ring 与 Bulk Mapping SHOULD 支持零拷贝或少拷贝路径。
4. **可撤销**：Root Court MUST 能撤销 corridor capability，并使后续访问失败。
5. **可观测**：Corridor SHOULD 暴露 trace point、计数器、状态与错误信息。
6. **可版本化**：Corridor 上层协议 MUST 有 protocol ID 与版本。

---

## 4. 非目标

RFC-0002 不定义以下内容：

1. 不定义完整 POSIX syscall ABI。
2. 不定义 Linux syscall 兼容层。
3. 不定义所有设备的具体协议。
4. 不定义完整网络协议栈 ABI。
5. 不强制所有庭使用同一种内部线程模型。
6. 不强制所有 Corridor 采用同步 RPC。
7. 不要求第一版支持跨机器分布式 Corridor。
8. 不要求第一版支持动态链接器 ABI。
9. 不承诺 capability handle 在不同 Court 之间数值相同。
10. 不承诺 Shared Ring 对所有负载都是最优传输。

---

## 5. ABI 分层

Court Kernel ABI 分为六层。

```text
L0  Machine Call Gate ABI
    x86-64 register convention, vmcall/vmmcall/syscall/trap, clobber rule

L1  Primitive Type ABI
    ck_u64, ck_cap_t, ck_rights_t, ck_status_t, struct layout, endianness

L2  Capability Invocation ABI
    ck_cap_invoke, ck_msg, op ID, error code, cap transfer

L3  Kernel Object ABI
    Root, CSpace, Namespace, Memory, Signal, Court, Corridor, DeviceQueue 等对象操作

L4  Corridor Transport ABI
    Control Channel, Shared Ring, Bulk Mapping, Device Queue, Signal/Doorbell

L5  Protocol / IDL ABI
    protocol ID, schema version, operation schema, required rights, compatibility rule
```

实验实现可以先落地 L0-L3，再实现 L4 的 Channel 与 Shared Ring，最后补 L5。

---

## 6. 基本类型与布局规则

### 6.1 固定宽度类型

ABI 使用固定宽度整数，不使用 C `long`、Rust `usize` 作为跨边界字段。

```c
typedef uint8_t   ck_u8;
typedef uint16_t  ck_u16;
typedef uint32_t  ck_u32;
typedef uint64_t  ck_u64;
typedef int64_t   ck_i64;

typedef ck_u64    ck_cap_t;
typedef ck_u64    ck_rights_t;
typedef ck_i64    ck_status_t;
typedef ck_u64    ck_vaddr_t;
typedef ck_u64    ck_paddr_token_t;  /* 不等于裸物理地址，除非 capability 明确授权 */
typedef ck_u64    ck_size_t;
typedef ck_u64    ck_offset_t;
```

Rust 对应定义：

```rust
pub type CkU8 = u8;
pub type CkU16 = u16;
pub type CkU32 = u32;
pub type CkU64 = u64;
pub type CkI64 = i64;

pub type CkCap = u64;
pub type CkRights = u64;
pub type CkStatus = i64;
pub type CkVaddr = u64;
pub type CkSize = u64;
pub type CkOffset = u64;
```

### 6.2 字节序

x86-64 ABI 字段默认使用 **little-endian**。

跨机器或跨架构版本 MAY 引入 network-endian profile，但不属于 RFC-0002 第一版范围。

### 6.3 对齐

1. ABI struct MUST 使用 8 字节对齐。
2. ABI struct SHOULD 避免 `packed`。
3. 所有结构体 MUST 包含显式保留字段，保留字段发送方必须置 0，接收方必须忽略未知保留字段。
4. Rust 结构体 MUST 使用 `#[repr(C)]`。
5. C 结构体 SHOULD 使用 `static_assert(sizeof(...))` 和 `alignof(...)` 验证。

### 6.4 指针规则

ABI 中的 pointer 字段 MUST 被视为 **调用方虚拟地址**，而不是 Root Court host pointer。

Root Court 读取 pointer 时 MUST：

```text
1. 验证该地址属于调用方 Courtlet 地址空间；
2. 验证长度不会溢出；
3. 验证访问方向符合输入/输出语义；
4. 对小消息使用 copy-in/copy-out；
5. 对大对象要求显式 MemoryRegionCap 或 pinned shared region；
6. 防止 TOCTOU：被复制的控制结构应一次性快照。
```

---

## 7. ABI 版本

### 7.1 版本常量

```c
#define CK_ABI_MAGIC        0x434b414249303032ULL /* "CKABI002" */
#define CK_ABI_MAJOR        0
#define CK_ABI_MINOR        2
#define CK_ABI_PATCH        0
```

语义：

```text
major: 不兼容 ABI 变更
minor: 向后兼容新增字段、新 operation、新 object type
patch: 文档、错误码说明、非破坏性澄清
```

### 7.2 版本发现

Courtlet MUST 从 `ck_boot_info` 读取 Root Court 支持的 ABI 版本。Courtlet MAY 调用：

```text
CK_OP_ROOT_QUERY_ABI
```

获取更完整的 feature bitmap。

### 7.3 结构体版本规则

每个跨 ABI 结构体 SHOULD 采用以下前缀：

```c
struct ck_header {
    ck_u64 magic;
    ck_u16 size;
    ck_u16 version;
    ck_u32 flags;
};
```

接收方处理结构体时：

```text
if size < minimal_known_size: return CK_ERR_ABI
if version.major unsupported: return CK_ERR_ABI
if unknown flags with MUST_UNDERSTAND bit: return CK_ERR_NOTSUP
otherwise: ignore unknown trailing fields
```

---

## 8. Capability Handle ABI

### 8.1 `ck_cap_t`

`ck_cap_t` 是 Courtlet 本地 capability handle。

```text
ck_cap_t 不等于对象指针。
ck_cap_t 不等于全局对象 ID。
ck_cap_t 只在持有它的 CSpace 中有意义。
```

推荐位布局：

```text
bits  0..31   slot_index
bits 32..47   generation
bits 48..55   type_hint
bits 56..63   local_flags
```

该布局是推荐实现，不是强制 wire contract。符合 RFC-0002 的实现只需保证：

```text
1. handle 是 64-bit；
2. 随机伪造 handle 不能获得权限；
3. 被撤销或 generation 不匹配的 handle 必须失败；
4. handle 数值不能跨 Court 直接解释。
```

### 8.2 Capability Rights

```c
#define CK_RIGHT_READ       (1ULL << 0)
#define CK_RIGHT_WRITE      (1ULL << 1)
#define CK_RIGHT_EXECUTE    (1ULL << 2)
#define CK_RIGHT_MAP        (1ULL << 3)
#define CK_RIGHT_SEND       (1ULL << 4)
#define CK_RIGHT_RECV       (1ULL << 5)
#define CK_RIGHT_MINT       (1ULL << 6)
#define CK_RIGHT_REVOKE     (1ULL << 7)
#define CK_RIGHT_DELEGATE   (1ULL << 8)
#define CK_RIGHT_OBSERVE    (1ULL << 9)
#define CK_RIGHT_CONFIGURE  (1ULL << 10)
#define CK_RIGHT_BIND       (1ULL << 11)
#define CK_RIGHT_SCHEDULE   (1ULL << 12)
#define CK_RIGHT_SIGNAL     (1ULL << 13)
#define CK_RIGHT_WAIT       (1ULL << 14)
#define CK_RIGHT_TRANSFER   (1ULL << 15)
#define CK_RIGHT_ADMIN      (1ULL << 63)
```

规则：

1. Object operation MUST 声明所需 rights。
2. Root Court MUST 在执行前检查 rights。
3. Capability delegation MUST NOT 增加原 capability 不具备的 rights。
4. Capability mint MAY 衰减 rights。
5. Revoke MUST 使后续 invocation 失败。
6. Observe right 不代表 control right。

### 8.3 Capability Object Type

```c
#define CK_OBJ_INVALID          0x0000
#define CK_OBJ_ROOT             0x0001
#define CK_OBJ_CSPACE           0x0002
#define CK_OBJ_NAMESPACE        0x0003
#define CK_OBJ_COURT            0x0004
#define CK_OBJ_COURTLET         0x0005
#define CK_OBJ_ADDRESS_SPACE    0x0006
#define CK_OBJ_MEMORY_REGION    0x0007
#define CK_OBJ_CPU_SET          0x0008
#define CK_OBJ_TIME_BUDGET      0x0009
#define CK_OBJ_IRQ              0x000a
#define CK_OBJ_DEVICE           0x000b
#define CK_OBJ_DEVICE_QUEUE     0x000c
#define CK_OBJ_CHANNEL          0x000d
#define CK_OBJ_SHARED_RING      0x000e
#define CK_OBJ_BULK_REGION      0x000f
#define CK_OBJ_SIGNAL           0x0010
#define CK_OBJ_TRACE            0x0011
#define CK_OBJ_POLICY           0x0012
#define CK_OBJ_IMAGE            0x0013
#define CK_OBJ_CORRIDOR         0x0014
```

---

## 9. 错误码 ABI

所有 Root Court invocation 返回 `ck_status_t`。

```c
#define CK_OK                  0
#define CK_ERR_INVAL          -1   /* 参数非法 */
#define CK_ERR_ACCESS         -2   /* 权限不足 */
#define CK_ERR_NOENT          -3   /* 对象不存在 */
#define CK_ERR_AGAIN          -4   /* 暂不可用，可重试 */
#define CK_ERR_NOMEM          -5   /* 内存不足 */
#define CK_ERR_BUSY           -6   /* 对象忙 */
#define CK_ERR_FAULT          -7   /* 调用方地址不可访问 */
#define CK_ERR_BADTYPE        -8   /* capability 类型不匹配 */
#define CK_ERR_NOSYS          -9   /* operation 未实现 */
#define CK_ERR_REVOKED        -10  /* capability 已撤销 */
#define CK_ERR_TIMEOUT        -11
#define CK_ERR_OVERFLOW       -12
#define CK_ERR_UNDERFLOW      -13
#define CK_ERR_ABI            -14  /* ABI 版本/布局/大小错误 */
#define CK_ERR_BADSTATE       -15
#define CK_ERR_CHECKSUM       -16
#define CK_ERR_NOTSUP         -17
#define CK_ERR_DEAD           -18  /* 对端死亡或 corridor dead */
#define CK_ERR_FULL           -19
#define CK_ERR_EMPTY          -20
#define CK_ERR_RETRY_SIGNAL   -21  /* 需要先通知/等待 */
```

错误码规则：

1. 正数保留给对象特定小返回值，不作为通用错误码。
2. `CK_OK` 表示成功。
3. 负数表示错误。
4. Rust safe wrapper SHOULD 转换为 `Result<T, CkError>`。
5. C wrapper SHOULD 保留原始 `ck_status_t`。

---

## 10. Machine Call Gate ABI

### 10.1 调用模式

Courtlet 到 Root Court 的 invocation 可以由以下 call gate 承载：

| 模式 | 入口 | 用途 |
|---|---|---|
| Trusted direct | 直接函数调用 | 早期 bring-up 与单地址空间测试 |
| Trap/syscall | `syscall` 或软件中断 | trusted kernel 原型或无 VMX 过渡期 |
| VMX | `vmcall` | Intel VMX non-root Courtlet |
| SVM | `vmmcall` | AMD SVM guest Courtlet |

语义 MUST 相同：

```text
ck_cap_invoke(target, op, msg, flags) -> status
```

### 10.2 x86-64 寄存器约定

标准 invocation 使用以下寄存器：

```text
RAX = CK_CALL_INVOKE
RDI = target capability handle
RSI = operation ID
RDX = guest virtual address of ck_msg
R10 = flags
R8  = reserved, must be 0 in RFC-0002
R9  = reserved, must be 0 in RFC-0002
```

返回：

```text
RAX = ck_status_t
RDX = optional result word, operation-specific；未使用时为 0
```

Clobber 规则：

```text
Caller-saved registers may be clobbered: RAX, RCX, RDX, RSI, RDI, R8-R11.
Callee-saved registers must be preserved by wrapper/runtime: RBX, RBP, R12-R15.
For syscall path, RCX and R11 are clobbered by architecture convention.
```

### 10.3 调用编号

```c
#define CK_CALL_INVOKE     0x434b0001U
#define CK_CALL_YIELD      0x434b0002U
#define CK_CALL_PANIC      0x434b0003U
#define CK_CALL_TRACE_FAST 0x434b0004U
#define CK_CALL_SIGNAL     0x434b0005U
```

`CK_CALL_INVOKE` 是唯一必须实现的入口。其他 fast call MAY 在性能路径中实现，但 MUST 有等价 `ck_cap_invoke` 路径。

### 10.4 C wrapper

```c
ck_status_t ck_cap_invoke(
    ck_cap_t target,
    ck_u32 op,
    struct ck_msg *msg,
    ck_u64 flags
);
```

### 10.5 Rust wrapper

```rust
#[repr(C)]
pub struct CkMsg {
    pub magic: u64,
    pub size: u16,
    pub version: u16,
    pub flags: u32,
    pub in_ptr: u64,
    pub in_len: u32,
    pub out_ptr: u64,
    pub out_len: u32,
    pub capv_ptr: u64,
    pub capv_count: u16,
    pub reserved0: u16,
    pub reserved1: u32,
}

extern "C" {
    pub fn ck_cap_invoke(
        target: CkCap,
        op: u32,
        msg: *mut CkMsg,
        flags: u64,
    ) -> CkStatus;
}
```

Rust safe wrapper SHOULD 不直接暴露裸 pointer：

```rust
pub fn invoke<T: AbiIn, U: AbiOut>(
    target: Cap,
    op: OpId,
    input: &T,
    output: &mut U,
    flags: InvokeFlags,
) -> Result<(), CkError>;
```

### 10.6 Assembly wrapper 示例

```asm
/* System V AMD64 ABI wrapper: ck_cap_invoke(target, op, msg, flags) */
.global ck_cap_invoke
ck_cap_invoke:
    /* arguments: rdi=target, rsi=op, rdx=msg, rcx=flags in C ABI */
    mov     %rcx, %r10
    mov     $0x434b0001, %rax
#ifdef CK_USE_VMCALL
    vmcall
#elif defined(CK_USE_VMMCALL)
    vmmcall
#else
    syscall
#endif
    ret
```

注意：真实实现必须根据构建目标处理 `syscall` MSR、VMX exit、错误恢复和 clobber 规则。上例只说明寄存器映射。

---

## 11. `ck_msg` ABI

### 11.1 消息结构

```c
#define CK_MSG_MAGIC 0x434b4d5347303032ULL /* "CKMSG002" */

struct ck_msg {
    ck_u64 magic;
    ck_u16 size;
    ck_u16 version;
    ck_u32 flags;

    ck_vaddr_t in_ptr;
    ck_u32     in_len;
    ck_vaddr_t out_ptr;
    ck_u32     out_len;

    ck_vaddr_t capv_ptr;
    ck_u16     capv_count;
    ck_u16     reserved0;
    ck_u32     reserved1;
};
```

### 11.2 Message flags

```c
#define CK_MSG_F_NONE            0
#define CK_MSG_F_CAP_TRANSFER    (1U << 0)
#define CK_MSG_F_CAP_MOVE        (1U << 1)
#define CK_MSG_F_NONBLOCK        (1U << 2)
#define CK_MSG_F_DEADLINE        (1U << 3)
#define CK_MSG_F_ATOMIC          (1U << 4)
#define CK_MSG_F_TRACE           (1U << 5)
#define CK_MSG_F_MUST_UNDERSTAND (1U << 31)
```

### 11.3 Capability vector

Capability 参数通过 `ck_cap_arg` 数组传递。

```c
struct ck_cap_arg {
    ck_cap_t cap;
    ck_rights_t required_rights;
    ck_u32 role;
    ck_u32 flags;
};
```

`role` 由 operation 或 IDL 定义。例如：

```text
0 = primary object
1 = input memory region
2 = output memory region
3 = signal endpoint
4 = reply channel
```

### 11.4 Copy-in/copy-out 规则

Root Court MUST 先 copy-in `ck_msg`，然后按需要 copy-in `in_ptr` 指向的数据。对于输出，Root Court MUST 在操作成功或定义允许的错误路径中 copy-out。

对于 `CK_MSG_F_ATOMIC`：

```text
成功时所有输出和 capability mutation 一起提交。
失败时不得留下部分 capability mutation。
```

---

## 12. Operation ID 空间

Operation ID 为 32-bit。

```text
0x0000_0000 - 0x0000_FFFF   Root / generic
0x0001_0000 - 0x0001_FFFF   Capability / CSpace
0x0002_0000 - 0x0002_FFFF   Namespace
0x0003_0000 - 0x0003_FFFF   Corridor generic
0x0004_0000 - 0x0004_FFFF   Memory / AddressSpace
0x0005_0000 - 0x0005_FFFF   CPU / Time / Scheduler
0x0006_0000 - 0x0006_FFFF   IRQ / Device / DeviceQueue
0x0007_0000 - 0x0007_FFFF   Trace / Observe
0x0008_0000 - 0x0008_FFFF   Court / Courtlet / Image
0x1000_0000 - 0x1FFF_FFFF   Protocol standard extensions
0x8000_0000 - 0xFFFF_FFFF   Experimental / vendor / research
```

### 12.1 Root generic ops

```c
#define CK_OP_ROOT_QUERY_ABI       0x00000001
#define CK_OP_ROOT_QUERY_FEATURES  0x00000002
#define CK_OP_ROOT_GET_TIME        0x00000003
#define CK_OP_ROOT_YIELD           0x00000004
#define CK_OP_ROOT_PANIC           0x00000005
```

### 12.2 Capability ops

```c
#define CK_OP_CAP_QUERY            0x00010001
#define CK_OP_CAP_DUP              0x00010002
#define CK_OP_CAP_MINT             0x00010003
#define CK_OP_CAP_REVOKE           0x00010004
#define CK_OP_CAP_TRANSFER         0x00010005
#define CK_OP_CSPACE_ALLOC         0x00010006
#define CK_OP_CSPACE_FREE          0x00010007
```

### 12.3 Namespace ops

```c
#define CK_OP_NS_LOOKUP            0x00020001
#define CK_OP_NS_OPEN              0x00020002
#define CK_OP_NS_BIND              0x00020003
#define CK_OP_NS_UNBIND            0x00020004
#define CK_OP_NS_WATCH             0x00020005
#define CK_OP_NS_STAT              0x00020006
```

### 12.4 Corridor ops

```c
#define CK_OP_CORRIDOR_CREATE      0x00030001
#define CK_OP_CORRIDOR_OPEN        0x00030002
#define CK_OP_CORRIDOR_CLOSE       0x00030003
#define CK_OP_CORRIDOR_STAT        0x00030004
#define CK_OP_CORRIDOR_SEAL        0x00030005
#define CK_OP_CORRIDOR_REVOKE      0x00030006
#define CK_OP_CHANNEL_SEND         0x00030020
#define CK_OP_CHANNEL_RECV         0x00030021
#define CK_OP_RING_MAP             0x00030040
#define CK_OP_RING_COMMIT          0x00030041
#define CK_OP_RING_NOTIFY          0x00030042
#define CK_OP_RING_DRAIN           0x00030043
#define CK_OP_BULK_GRANT           0x00030060
#define CK_OP_BULK_REVOKE          0x00030061
#define CK_OP_SIGNAL_RAISE         0x00030080
#define CK_OP_SIGNAL_WAIT          0x00030081
#define CK_OP_SIGNAL_POLL          0x00030082
```

### 12.5 Memory ops

```c
#define CK_OP_MEM_ALLOC            0x00040001
#define CK_OP_MEM_MAP              0x00040002
#define CK_OP_MEM_UNMAP            0x00040003
#define CK_OP_MEM_SHARE            0x00040004
#define CK_OP_MEM_PIN              0x00040005
#define CK_OP_MEM_UNPIN            0x00040006
#define CK_OP_ASPACE_QUERY         0x00040020
```

### 12.6 Court ops

```c
#define CK_OP_COURT_QUERY          0x00080001
#define CK_OP_COURT_CREATE         0x00080002
#define CK_OP_COURT_START          0x00080003
#define CK_OP_COURT_STOP           0x00080004
#define CK_OP_COURT_KILL           0x00080005
#define CK_OP_COURT_RESTART        0x00080006
#define CK_OP_COURT_SET_PLACEMENT  0x00080007
#define CK_OP_IMAGE_LOAD           0x00080020
```

---

## 13. Boot ABI

### 13.1 Courtlet entry point

Courtlet image 的入口 SHOULD 使用以下签名：

```c
typedef void (*ck_courtlet_entry_t)(const struct ck_boot_info *boot_info);
```

Rust：

```rust
#[no_mangle]
pub extern "C" fn ck_courtlet_entry(boot: *const CkBootInfo) -> ! {
    /* initialize runtime */
    loop {}
}
```

### 13.2 入口寄存器

x86-64 Courtlet entry：

```text
RDI = guest virtual address of ck_boot_info
RSP = initial stack top, 16-byte aligned before call frame convention
RFLAGS.IF = 0 by default unless boot flag enables virtual interrupt delivery
其他通用寄存器未定义，Courtlet 不得依赖
```

### 13.3 `ck_boot_info`

```c
#define CK_BOOT_MAGIC 0x434b424f4f543032ULL /* "CKBOOT02" */

struct ck_boot_info {
    ck_u64 magic;
    ck_u16 size;
    ck_u16 version;
    ck_u32 flags;

    ck_u16 abi_major;
    ck_u16 abi_minor;
    ck_u16 abi_patch;
    ck_u16 arch;

    ck_u64 court_id;
    ck_u64 courtlet_id;

    ck_cap_t root_cap;
    ck_cap_t self_court_cap;
    ck_cap_t self_courtlet_cap;
    ck_cap_t cspace_cap;
    ck_cap_t namespace_cap;
    ck_cap_t address_space_cap;
    ck_cap_t initial_signal_cap;

    ck_vaddr_t manifest_ptr;
    ck_u32     manifest_len;
    ck_u32     reserved0;

    ck_vaddr_t initial_caps_ptr;
    ck_u32     initial_caps_count;
    ck_u32     reserved1;

    ck_u64 tsc_frequency_hz;
    ck_u64 feature_bitmap_low;
    ck_u64 feature_bitmap_high;
};
```

### 13.4 Initial capability table

```c
struct ck_initial_cap {
    ck_u32 name_id;
    ck_u32 object_type;
    ck_rights_t rights;
    ck_cap_t cap;
};
```

推荐 `name_id`：

```text
1 = root
2 = self_court
3 = self_courtlet
4 = cspace
5 = namespace
6 = address_space
7 = initial_signal
8 = log_channel
9 = trace
10 = policy_view
```

### 13.5 Boot feature bitmap

```c
#define CK_FEATURE_VMX             (1ULL << 0)
#define CK_FEATURE_EPT             (1ULL << 1)
#define CK_FEATURE_SVM             (1ULL << 2)
#define CK_FEATURE_NPT             (1ULL << 3)
#define CK_FEATURE_IOMMU           (1ULL << 4)
#define CK_FEATURE_X2APIC          (1ULL << 5)
#define CK_FEATURE_HFI             (1ULL << 6)
#define CK_FEATURE_PCID            (1ULL << 7)
#define CK_FEATURE_INVPCID         (1ULL << 8)
#define CK_FEATURE_CET             (1ULL << 9)
#define CK_FEATURE_PKU             (1ULL << 10)
#define CK_FEATURE_RDT_CAT         (1ULL << 11)
#define CK_FEATURE_TRUSTED_BRINGUP (1ULL << 63)
```

---

## 14. Namespace ABI

Namespace ABI 提供发现，不直接提供权限。

### 14.1 Path encoding

1. Path MUST 使用 UTF-8。
2. Path MUST 使用 `/` 分隔。
3. Path MUST NOT 包含 NUL 字节。
4. Path SHOULD 使用小写 ASCII、数字、`-`、`_`、`.`。
5. Path lookup 结果 MUST 受调用方 namespace view 限制。

### 14.2 `CK_OP_NS_LOOKUP`

输入：

```c
struct ck_ns_lookup_in {
    ck_u64 path_ptr;
    ck_u32 path_len;
    ck_u32 flags;
};
```

输出：

```c
struct ck_ns_lookup_out {
    ck_u64 object_id;        /* opaque, only for observe/stat */
    ck_u32 object_type;
    ck_u32 required_open_flags;
    ck_u64 protocol_id_hi;
    ck_u64 protocol_id_lo;
};
```

规则：

```text
lookup 成功不等于获得权限。
lookup 输出不得包含可直接调用的 capability，除非调用者已有 namespace open right 且 operation 是 NS_OPEN。
```

### 14.3 `CK_OP_NS_OPEN`

输入：

```c
struct ck_ns_open_in {
    ck_u64 path_ptr;
    ck_u32 path_len;
    ck_u32 flags;
    ck_rights_t requested_rights;
};
```

输出：

```c
struct ck_ns_open_out {
    ck_cap_t cap;
    ck_u32 object_type;
    ck_u32 granted_flags;
    ck_rights_t granted_rights;
};
```

Root Court MUST 验证：

```text
1. 调用者 namespace_cap 是否允许 lookup/open；
2. path 是否在调用者 namespace view 内；
3. 目标对象 policy 是否允许授予 requested_rights；
4. 返回 capability rights 必须小于等于 requested_rights 和 policy allowance。
```

### 14.4 `CK_OP_NS_BIND`

需要 `CK_RIGHT_BIND`。

输入：

```c
struct ck_ns_bind_in {
    ck_u64 path_ptr;
    ck_u32 path_len;
    ck_u32 flags;
    ck_cap_t object_cap;
    ck_rights_t default_open_rights;
    ck_u64 protocol_id_hi;
    ck_u64 protocol_id_lo;
};
```

`default_open_rights` 只是 policy 输入，不意味着所有 caller 都能得到这些 rights。

---

## 15. Corridor 总体 ABI

### 15.1 Corridor 类型

```c
#define CK_CORRIDOR_CONTROL_CHANNEL  1
#define CK_CORRIDOR_SHARED_RING      2
#define CK_CORRIDOR_BULK_MAPPING     3
#define CK_CORRIDOR_DEVICE_QUEUE     4
#define CK_CORRIDOR_SIGNAL           5
```

### 15.2 Corridor state

```c
#define CK_CORRIDOR_INIT       0
#define CK_CORRIDOR_READY      1
#define CK_CORRIDOR_DRAINING   2
#define CK_CORRIDOR_REVOKING   3
#define CK_CORRIDOR_REVOKED    4
#define CK_CORRIDOR_DEAD       5
```

状态机：

```text
INIT -> READY -> DRAINING -> REVOKING -> REVOKED
READY -> DEAD
DRAINING -> DEAD
REVOKING -> DEAD
```

规则：

1. `READY` 状态允许正常传输。
2. `DRAINING` 状态允许消费已提交数据，但不允许新提交。
3. `REVOKING` 状态 Root Court 正在撤销 mapping/queue/signal。
4. `REVOKED` 状态后所有新 operation MUST 返回 `CK_ERR_REVOKED`。
5. `DEAD` 表示对端死亡或不可恢复错误。

### 15.3 Corridor descriptor

```c
#define CK_CORRIDOR_MAGIC 0x434b434f52523032ULL /* "CKCORR02" */

struct ck_corridor_desc {
    ck_u64 magic;
    ck_u16 size;
    ck_u16 version;
    ck_u32 flags;

    ck_u64 corridor_id;
    ck_u32 kind;
    ck_u32 state;

    ck_u64 protocol_id_hi;
    ck_u64 protocol_id_lo;
    ck_u16 protocol_major;
    ck_u16 protocol_minor;
    ck_u16 protocol_patch;
    ck_u16 endpoint_role;

    ck_u64 qos_latency_ns_p99;
    ck_u64 qos_bandwidth_bytes_s;
    ck_u64 capacity;
    ck_u64 mtu;

    ck_cap_t primary_transport_cap;
    ck_cap_t signal_cap;
    ck_cap_t trace_cap;
};
```

### 15.4 Endpoint role

```c
#define CK_ENDPOINT_CLIENT      1
#define CK_ENDPOINT_SERVER      2
#define CK_ENDPOINT_PRODUCER    3
#define CK_ENDPOINT_CONSUMER    4
#define CK_ENDPOINT_BIDIR       5
#define CK_ENDPOINT_OBSERVER    6
```

---

## 16. Control Channel ABI

Control Channel 用于配置、状态查询、低频 RPC、capability transfer 和协议协商。

### 16.1 Channel message header

```c
#define CK_CHANNEL_MSG_MAGIC 0x434b43484d303032ULL /* "CKCHM002" */

struct ck_channel_msg_header {
    ck_u64 magic;
    ck_u16 size;
    ck_u16 version;
    ck_u32 flags;

    ck_u64 corridor_id;
    ck_u64 seq;
    ck_u64 reply_to;

    ck_u64 protocol_id_hi;
    ck_u64 protocol_id_lo;
    ck_u32 type_id;
    ck_u32 body_len;

    ck_u16 cap_count;
    ck_u16 reserved0;
    ck_u32 reserved1;
};
```

### 16.2 Channel send

`CK_OP_CHANNEL_SEND` 输入：

```c
struct ck_channel_send_in {
    ck_u64 msg_ptr;
    ck_u32 msg_len;
    ck_u32 flags;
    ck_u64 deadline_ns;
};
```

`msg_ptr` 指向：

```text
ck_channel_msg_header
body bytes
optional ck_cap_arg[cap_count]
```

需要 rights：

```text
Channel cap: CK_RIGHT_SEND
Transferred caps: CK_RIGHT_DELEGATE or CK_RIGHT_TRANSFER depending mode
```

### 16.3 Channel recv

`CK_OP_CHANNEL_RECV` 输入：

```c
struct ck_channel_recv_in {
    ck_u64 buf_ptr;
    ck_u32 buf_len;
    ck_u32 flags;
    ck_u64 deadline_ns;
};
```

输出：

```c
struct ck_channel_recv_out {
    ck_u32 bytes_written;
    ck_u32 cap_count;
    ck_u64 seq;
};
```

### 16.4 Channel 顺序语义

1. 每个 Channel endpoint SHOULD 保证 FIFO。
2. Capability transfer MUST 与 message body 原子提交。
3. 如果 body 成功接收但 cap transfer 失败，则整个 receive MUST 失败或消息 MUST 被销毁，并返回 `CK_ERR_ACCESS` 或 `CK_ERR_BADSTATE`。
4. Channel MAY 支持 nonblocking；无消息时返回 `CK_ERR_EMPTY`。

---

## 17. Shared Ring ABI

Shared Ring 用于高吞吐、低延迟、固定方向的数据传输。RFC-0002 baseline 定义 **unidirectional SPSC ring**。MPSC/MPMC 是后续扩展。

### 17.1 Ring memory model

1. Ring memory MUST 由 MemoryRegionCap 显式授权。
2. Producer 与 Consumer 对 ring header 和 entries 的访问权限 SHOULD 分离。
3. Producer 只能写 producer-owned fields 和待提交 entry。
4. Consumer 只能写 consumer-owned fields。
5. Root Court MAY 使用 EPT/IOMMU/PTE 权限把 header/entry 区域分段映射。

### 17.2 Ring header

```c
#define CK_RING_MAGIC 0x434b52494e473032ULL /* "CKRING02" */

struct ck_ring_header {
    ck_u64 magic;
    ck_u16 size;
    ck_u16 version;
    ck_u32 flags;

    ck_u64 corridor_id;
    ck_u32 state;
    ck_u32 entry_size;

    ck_u64 capacity;       /* power of two */
    ck_u64 producer_idx;   /* monotonically increasing */
    ck_u64 consumer_idx;   /* monotonically increasing */

    ck_u64 producer_event_idx;
    ck_u64 consumer_event_idx;

    ck_u64 dropped_count;
    ck_u64 error_count;
    ck_u64 reserved[8];
};
```

### 17.3 Ring flags

```c
#define CK_RING_F_SPSC              (1U << 0)
#define CK_RING_F_MPSC              (1U << 1)  /* extension */
#define CK_RING_F_MPMC              (1U << 2)  /* extension */
#define CK_RING_F_EVENT_IDX         (1U << 3)
#define CK_RING_F_ZERO_COPY         (1U << 4)
#define CK_RING_F_DROP_ON_FULL      (1U << 5)
#define CK_RING_F_BLOCK_ON_FULL     (1U << 6)
#define CK_RING_F_CHECKSUM          (1U << 7)
#define CK_RING_F_MUST_UNDERSTAND   (1U << 31)
```

### 17.4 Ring descriptor

```c
struct ck_ring_desc {
    ck_u64 region_id;      /* opaque shared region id */
    ck_u64 offset;         /* offset inside region */
    ck_u32 len;            /* bytes valid */
    ck_u32 capacity;       /* bytes available in backing buffer */
    ck_u16 kind;           /* protocol-specific */
    ck_u16 flags;
    ck_u32 rights;         /* descriptor-local access hint */
    ck_u64 seq;
    ck_u64 meta0;
    ck_u64 meta1;
};
```

### 17.5 Ring producer algorithm

伪代码：

```rust
fn ring_push(ring: &Ring, desc: CkRingDesc) -> Result<(), CkError> {
    let prod = ring.producer_idx.load(Relaxed);
    let cons = ring.consumer_idx.load(Acquire);

    if prod - cons == ring.capacity {
        return Err(CkError::Full);
    }

    let slot = prod & (ring.capacity - 1);
    ring.entries[slot] = desc;

    fence(Release);
    ring.producer_idx.store(prod + 1, Release);

    if ring.should_notify_consumer(prod + 1) {
        ring.notify_consumer()?;
    }

    Ok(())
}
```

### 17.6 Ring consumer algorithm

```rust
fn ring_pop(ring: &Ring) -> Result<CkRingDesc, CkError> {
    let cons = ring.consumer_idx.load(Relaxed);
    let prod = ring.producer_idx.load(Acquire);

    if cons == prod {
        return Err(CkError::Empty);
    }

    let slot = cons & (ring.capacity - 1);
    let desc = ring.entries[slot];

    fence(Release);
    ring.consumer_idx.store(cons + 1, Release);

    if ring.should_notify_producer(cons + 1) {
        ring.notify_producer()?;
    }

    Ok(desc)
}
```

### 17.7 Memory ordering

虽然 x86-64 具有较强的 TSO 语义，ABI 仍规定以 C11/Rust atomic 语义表达：

```text
Producer 写 entry -> Release store producer_idx
Consumer Acquire load producer_idx -> 读 entry
Consumer 释放 slot -> Release store consumer_idx
Producer Acquire load consumer_idx -> 判断空间
```

实现 MUST NOT 依赖编译器不会重排序。Rust/C 代码必须使用 atomic 或明确 fence。

### 17.8 Ring full/empty 规则

```text
full  iff producer_idx - consumer_idx == capacity
empty iff producer_idx == consumer_idx
slot  = index & (capacity - 1)
```

`capacity` MUST 是 2 的幂，且 MUST 大于 1。

### 17.9 Notification suppression

如果 `CK_RING_F_EVENT_IDX` 设置：

```text
consumer_event_idx 表示 consumer 希望 producer 在 producer_idx 到达该值时通知。
producer_event_idx 表示 producer 希望 consumer 在 consumer_idx 到达该值时通知。
```

事件索引只影响通知，不影响数据可见性。

### 17.10 Revocation

Root Court 撤销 SharedRingCap 时 MUST：

```text
1. 将 ring state 置为 REVOKING 或 REVOKED；
2. 阻止新的 CK_OP_RING_MAP；
3. 根据 policy 选择 drain 或 immediate revoke；
4. 撤销相关 EPT/PTE mapping；
5. 使后续 ring operation 返回 CK_ERR_REVOKED；
6. 记录 trace event。
```

---

## 18. Bulk Mapping ABI

Bulk Mapping 用于共享大对象，例如文件页、视频帧、模型权重、大块日志缓冲区。

### 18.1 Bulk grant

```c
struct ck_bulk_grant_in {
    ck_cap_t memory_cap;
    ck_u64 offset;
    ck_u64 len;
    ck_rights_t rights;
    ck_u64 ttl_ns;
    ck_u32 flags;
    ck_u32 reserved;
};

struct ck_bulk_grant_out {
    ck_cap_t bulk_cap;
    ck_u64 grant_id;
};
```

规则：

1. Grant rights MUST 是 memory_cap rights 的子集。
2. TTL 为 0 表示无时间限制，但仍可被 revoke。
3. Bulk cap 可以通过 Channel 发送给另一端。
4. Bulk cap 的 map 必须通过 `CK_OP_MEM_MAP` 或 corridor-specific map 完成。

### 18.2 Copy-on-grant

如果调用方设置 `CK_BULK_F_COPY_ON_GRANT`，Root Court MAY 创建快照 region。第一版可以返回 `CK_ERR_NOTSUP`。

---

## 19. Device Queue ABI

Device Queue 是设备快路径连廊，用于 NIC queue、NVMe queue、GPU broker queue、virtio queue 等。

### 19.1 原则

1. Device Queue MUST 由 DeviceQueueCap 授权。
2. DMA-capable queue MUST 受 IOMMU 约束。
3. IRQ/MSI-X delivery MUST 由 IrqCap 或 SignalCap 表达。
4. Device Queue ABI 只定义 envelope，不定义具体设备协议。

### 19.2 Device queue descriptor

```c
struct ck_device_queue_desc {
    ck_u64 device_id;
    ck_u32 device_class;
    ck_u32 queue_index;

    ck_u64 protocol_id_hi;
    ck_u64 protocol_id_lo;

    ck_cap_t mmio_cap;
    ck_cap_t dma_region_cap;
    ck_cap_t irq_cap;
    ck_cap_t signal_cap;

    ck_u64 queue_depth;
    ck_u64 flags;
};
```

### 19.3 Device Queue 权限

| 操作 | 需要 rights |
|---|---|
| 配置 queue | `CK_RIGHT_CONFIGURE` |
| 映射 MMIO | `CK_RIGHT_MAP` + `CK_RIGHT_READ/WRITE` |
| 绑定 DMA region | `CK_RIGHT_MAP` |
| 绑定 IRQ | `CK_RIGHT_BIND` |
| 发 doorbell | `CK_RIGHT_SIGNAL` |
| 观测计数器 | `CK_RIGHT_OBSERVE` |

---

## 20. Signal / Doorbell ABI

Signal 用于轻量事件通知。

### 20.1 Signal 类型

```c
#define CK_SIGNAL_EDGE       1
#define CK_SIGNAL_LEVEL      2
#define CK_SIGNAL_COUNTED    3
#define CK_SIGNAL_TIMER      4
```

### 20.2 Signal raise

```c
struct ck_signal_raise_in {
    ck_u64 value;
    ck_u32 flags;
    ck_u32 reserved;
};
```

需要 `CK_RIGHT_SIGNAL`。

### 20.3 Signal wait

```c
struct ck_signal_wait_in {
    ck_u64 expected;
    ck_u64 timeout_ns;
    ck_u32 flags;
    ck_u32 reserved;
};

struct ck_signal_wait_out {
    ck_u64 observed;
    ck_u64 timestamp_ns;
};
```

需要 `CK_RIGHT_WAIT`。

### 20.4 Signal 语义

1. Edge signal MAY 合并多次通知。
2. Counted signal MUST 不丢失计数，除非溢出并记录 `error_count`。
3. Timer signal MUST 以 Root Court 时间源为准。
4. Signal 不携带大数据；大数据必须通过 Channel、Ring 或 Bulk Mapping 传输。

---

## 21. IDL 与 Protocol ABI

Corridor 上层协议 MUST 可识别、可版本化、可声明 capability rights。

### 21.1 Protocol ID

Protocol ID 为 128-bit：

```text
protocol_id = stable 128-bit identifier
```

生成方式可以是：

```text
1. 随机 UUIDv4；或
2. namespace path + protocol name + major version 的 hash；或
3. RFC 分配的固定编号。
```

第一版实验推荐使用固定 UUID 文本写入 manifest，再由构建工具生成 hi/lo。

### 21.2 协议版本

```text
major: 不兼容变更
minor: 向后兼容新增 message/op/field
patch: 非语义变更
```

### 21.3 IDL 示例

```text
protocol court.net.packet_tx v0.1 {
  id = "2f5089b9-7b26-4f5b-9b55-8e7d6e0a0001";
  transport = shared_ring;

  rights {
    producer: send | signal;
    consumer: recv | wait;
    observer: observe;
  }

  descriptor PacketDesc {
    region_id: u64;
    offset: u64;
    len: u32;
    flags: u32;
    flow_id: u64;
    timestamp_ns: u64;
  }
}
```

### 21.4 IDL 到 Rust/C

IDL 编译器 SHOULD 生成：

```text
1. C header: protocol constants + repr-compatible structs
2. Rust no_std crate: repr(C) structs + safe wrappers
3. ABI layout tests
4. manifest fragment
5. trace schema
```

Rust 生成代码 MUST 默认 `#![no_std]`。

---

## 22. Rust / C / Assembly 边界

### 22.1 Rust

Rust 是默认实现语言。ABI 边界要求：

```text
MUST use #[repr(C)] for ABI structs.
MUST NOT expose Rust enum layout across ABI.
MUST NOT expose trait object, slice, Vec, String, Box across ABI.
MUST wrap unsafe pointer operations in audited modules.
SHOULD use core::sync::atomic for ring indices.
SHOULD provide safe wrappers for cap invocation and corridor operations.
```

Root Court 与 Courtlet runtime 默认使用：

```rust
#![no_std]
```

需要 heap 的庭 MAY 使用 `alloc`，但 allocator capability 必须由 Root Court 或庭内 runtime 显式初始化。

### 22.2 C

C 用于：

```text
1. ABI header；
2. firmware/UEFI 适配；
3. C ecosystem driver shim；
4. 与外部工具链兼容；
5. bring-up 阶段快速验证。
```

C 代码 MUST 使用固定宽度整数。C ABI header SHOULD 是 Rust binding 的来源之一，但不应该让 bindgen 生成物成为唯一规范。

### 22.3 Assembly

Assembly 限于：

```text
boot entry
long mode transition
AP trampoline
interrupt/exception entry
context switch
VMX/SVM transition
ck_cap_invoke call gate wrapper
特殊寄存器读写薄封装
```

Assembly MUST 尽量薄，并把复杂逻辑移交 Rust/C。

---

## 23. Capability Transfer ABI

Capability transfer 是 Control Channel 和 Root invocation 的重要功能。

### 23.1 Transfer mode

```c
#define CK_CAP_XFER_COPY       1  /* duplicate/delegate */
#define CK_CAP_XFER_MOVE       2  /* sender loses cap */
#define CK_CAP_XFER_MINT       3  /* attenuate rights */
```

### 23.2 Transfer record

```c
struct ck_cap_transfer_record {
    ck_cap_t source_cap;
    ck_rights_t requested_rights;
    ck_u32 mode;
    ck_u32 role;
    ck_cap_t out_receiver_cap; /* receiver-side handle, filled by Root Court */
};
```

### 23.3 Transfer validation

Root Court MUST 验证：

```text
COPY requires CK_RIGHT_DELEGATE or object policy permitting duplicate.
MOVE requires CK_RIGHT_TRANSFER or ownership policy.
MINT requires CK_RIGHT_MINT and requested_rights subset of source rights.
Receiver CSpace must have free slot.
Target object's policy must allow receiver Court to hold this cap.
```

---

## 24. Security ABI Rules

### 24.1 No ambient authority

任何 Courtlet 不得因为“位置靠近硬件”而自动获得所有 Root operation 权限。所有操作必须通过 capability 检查。

### 24.2 No raw physical address by default

ABI 不得默认暴露裸物理地址。需要设备 DMA 或 MMIO 时，必须通过：

```text
MemoryRegionCap
DeviceCap
DeviceQueueCap
IrqCap
IOMMU policy
```

共同授权。

### 24.3 Pointer validation

Root Court MUST NOT 直接解引用 Courtlet 提供的虚拟地址。所有 guest pointer 都必须经过地址空间检查与 copy-in/copy-out 或受控 mapping。

### 24.4 Revocation ordering

Capability revoke 的可见性顺序：

```text
1. Root Court 标记 cap tree revoked；
2. 新 invocation 失败；
3. 相关 shared mapping / queue / IRQ 被解绑；
4. 对端收到 revoke/dead signal；
5. Trace 记录 revoke complete。
```

### 24.5 Time-of-check / time-of-use

控制结构必须 copy-in 快照。Shared memory 只用于被声明为共享的数据面，不用于 Root Court 可信控制参数，除非该结构通过 pinned immutable region 或 sequence lock 校验。

### 24.6 Protocol rights check

IDL 中声明的 required rights MUST 被生成代码与 Root Court runtime 双重检查：

```text
client wrapper: early fail
Root Court: authoritative fail
```

---

## 25. Trace 与 Observability ABI

### 25.1 Trace event

```c
struct ck_trace_event {
    ck_u64 timestamp_ns;
    ck_u64 source_court_id;
    ck_u64 corridor_id;
    ck_u32 event_type;
    ck_u32 flags;
    ck_u64 arg0;
    ck_u64 arg1;
    ck_u64 arg2;
    ck_u64 arg3;
};
```

### 25.2 Trace event types

```c
#define CK_TRACE_CAP_INVOKE        1
#define CK_TRACE_CAP_REVOKE        2
#define CK_TRACE_NS_LOOKUP         3
#define CK_TRACE_NS_OPEN           4
#define CK_TRACE_CHANNEL_SEND      5
#define CK_TRACE_CHANNEL_RECV      6
#define CK_TRACE_RING_PUSH         7
#define CK_TRACE_RING_POP          8
#define CK_TRACE_RING_FULL         9
#define CK_TRACE_RING_EMPTY        10
#define CK_TRACE_CORRIDOR_DEAD     11
#define CK_TRACE_COURT_START       12
#define CK_TRACE_COURT_STOP        13
#define CK_TRACE_DEVICE_IRQ        14
```

### 25.3 Observability rights

读取 trace 需要 `CK_RIGHT_OBSERVE`。Observe right MUST NOT 授予修改 corridor 或对象状态的能力。

---

## 26. ABI Compliance Test

RFC-0002 合规实现 SHOULD 提供以下测试。

### 26.1 Layout tests

C：

```c
static_assert(sizeof(struct ck_msg) == 64, "ck_msg size mismatch");
static_assert(_Alignof(struct ck_msg) == 8, "ck_msg align mismatch");
```

Rust：

```rust
const _: () = assert!(core::mem::size_of::<CkMsg>() == 64);
const _: () = assert!(core::mem::align_of::<CkMsg>() == 8);
```

### 26.2 Invocation tests

```text
1. invalid cap -> CK_ERR_NOENT or CK_ERR_REVOKED
2. wrong type -> CK_ERR_BADTYPE
3. missing rights -> CK_ERR_ACCESS
4. unknown op -> CK_ERR_NOSYS
5. bad message pointer -> CK_ERR_FAULT
6. ABI size too small -> CK_ERR_ABI
7. reserved nonzero with strict flag -> CK_ERR_ABI
```

### 26.3 Namespace tests

```text
1. lookup existing path succeeds
2. lookup missing path returns CK_ERR_NOENT
3. open without rights returns CK_ERR_ACCESS
4. bind without CK_RIGHT_BIND returns CK_ERR_ACCESS
5. private namespace view hides external path
```

### 26.4 Ring tests

```text
1. empty ring pop -> CK_ERR_EMPTY
2. full ring push -> CK_ERR_FULL
3. producer_idx and consumer_idx monotonic
4. wrap-around works at power-of-two capacity
5. descriptor data visible after Release/Acquire ordering
6. revoke during traffic returns CK_ERR_REVOKED or CK_ERR_DEAD
```

### 26.5 Capability transfer tests

```text
1. mint attenuates rights
2. mint cannot add rights
3. move removes sender capability
4. transfer to full receiver CSpace fails atomically
5. revoked source cap cannot be transferred
```

---

## 27. 最低合规实现

一个符合 RFC-0002 Draft 0.1 的最小实现 MUST 支持：

```text
1. ck_boot_info
2. ck_cap_t / ck_rights_t / ck_status_t
3. ck_cap_invoke
4. CK_OP_ROOT_QUERY_ABI
5. CK_OP_CAP_QUERY
6. CK_OP_CAP_MINT
7. CK_OP_CAP_REVOKE
8. CK_OP_NS_LOOKUP
9. CK_OP_NS_OPEN
10. CK_OP_CHANNEL_SEND
11. CK_OP_CHANNEL_RECV
12. CK_OP_RING_MAP
13. CK_OP_RING_NOTIFY
14. CK_OP_SIGNAL_RAISE
15. CK_OP_SIGNAL_WAIT
16. CK_OK and all required generic error codes
17. C header + Rust repr(C) bindings + layout tests
```

第一版实验可以暂不支持：

```text
Device Queue
Bulk Mapping
MPSC/MPMC Ring
Dynamic Protocol ID registry
IOMMU-backed DMA queue
Full revocation graph GC
```

但必须在 feature bitmap 中标记未支持。

---

## 28. 推荐代码组织

```text
court-kernel/
  rfc/
    RFC-0001.md
    RFC-0002.md
  abi/
    include/ck/abi.h
    rust/ck-abi/src/lib.rs
    tests/layout.rs
    tests/layout.c
  root/
    src/cap/
    src/ns/
    src/invoke/
    src/corridor/
    src/arch/x86_64/
  courtlets/
    net/
    app/
    crypto/
  tools/
    court-idl/
    manifestc/
```

### 28.1 ABI source of truth

推荐策略：

```text
1. abi/include/ck/abi.h 与 abi/rust/ck-abi/src/lib.rs 同步维护；
2. 使用 layout tests 确认二者一致；
3. 长期引入 court-idl 生成 C/Rust 绑定；
4. 禁止手写多个不一致 ABI 副本。
```

---

## 29. 示例：App Court 向 Net Court 发包

### 29.1 Namespace discovery

```text
App Court:
  ns.lookup("/court/net0/packet/tx")
  ns.open("/court/net0/packet/tx", SEND | SIGNAL)
  -> SharedRingCap + SignalCap
```

### 29.2 Ring mapping

```text
App Court:
  CK_OP_RING_MAP(SharedRingCap)
  -> maps ck_ring_header + ck_ring_desc[] + packet pool view
```

### 29.3 Packet push

```text
App Court writes packet bytes into granted packet buffer.
App Court writes ck_ring_desc.
App Court Release-stores producer_idx.
App Court raises SignalCap if event_idx requests notification.
```

### 29.4 Net Court consume

```text
Net Court waits on SignalCap or polls ring.
Net Court Acquire-loads producer_idx.
Net Court reads descriptor and packet bytes.
Net Court submits packet to DeviceQueue or software stack.
Net Court Release-stores consumer_idx.
```

### 29.5 Revocation

```text
Policy Court revokes App Court tx right.
Root Court marks SharedRingCap revoked.
New push fails.
Existing ring mapping is drained or unmapped according to policy.
Trace records CAP_REVOKE and CORRIDOR_REVOKED.
```

---

## 30. 示例：Crypto Court 签名服务

### 30.1 Control Channel protocol

```text
/court/crypto0/sign/ed25519
transport = control_channel
protocol = court.crypto.sign.v0
```

### 30.2 Message

```c
struct crypto_sign_request {
    ck_u64 key_id;
    ck_u64 msg_region_id;
    ck_u64 msg_offset;
    ck_u32 msg_len;
    ck_u32 flags;
};

struct crypto_sign_reply {
    ck_u8 signature[64];
    ck_u32 status;
    ck_u32 reserved;
};
```

### 30.3 Security rule

App Court 不获得 key material。它只获得 Sign Channel capability。

```text
App Court can send sign request.
Crypto Court validates policy.
Crypto Court returns signature.
Key never leaves Crypto Court.
```

---

## 31. 开放问题

1. `ck_cap_t` 是否应固定推荐位布局，还是完全 opaque？
2. Ring baseline 是否应直接采用 split-ring 三区域结构，还是保留当前简化 SPSC descriptor ring？
3. Capability revocation 是否需要同步阻塞直到所有 mapping 删除完成？
4. Device Queue ABI 是否需要单独 RFC-0003？
5. IDL 是否采用自研 court-idl，还是复用 FIDL/CapDL/Smithy-like schema？
6. Protocol ID 是否使用 UUID、hash，还是中心化分配？
7. Rust safe wrapper 是否作为 ABI 一部分，还是只是 SDK？
8. 是否需要定义 `ck_vdso`，为时间、CPU id、fast trace 提供无 trap 快路径？
9. MPSC/MPMC ring 是否进入 RFC-0002 后续小版本？
10. 与 future POSIX Court 的 syscall translation ABI 如何对接？

---

## 32. 参考依据

以下资料不是 Court Kernel 的依赖实现，但为本 RFC 的 ABI 选择提供参考：

1. Intel, *Intel® 64 and IA-32 Architectures Software Developer’s Manuals*.  
   https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html

2. System V AMD64 ABI, *System V Application Binary Interface AMD64 Architecture Processor Supplement*.  
   https://refspecs.linuxbase.org/elf/x86_64-abi-0.99.pdf

3. Rust Embedded Working Group, *The Embedded Rust Book: no_std*.  
   https://docs.rust-embedded.org/book/intro/no-std.html

4. Rust Reference, *Inline Assembly*.  
   https://doc.rust-lang.org/reference/inline-assembly.html

5. seL4 Documentation, *Capabilities*.  
   https://docs.sel4.systems/Tutorials/capabilities.html

6. Fuchsia Documentation, *Zircon Kernel Concepts: Handles and Rights*.  
   https://fuchsia.dev/fuchsia-src/concepts/kernel/concepts

7. Fuchsia Documentation, *Zircon Handles*.  
   https://fuchsia.dev/fuchsia-src/concepts/kernel/handles

8. Fuchsia Documentation, *System Interface*.  
   https://fuchsia.dev/fuchsia-src/concepts/kernel/system

9. OASIS, *Virtual I/O Device (VIRTIO) Version 1.3*.  
   https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html

10. DPDK Project, *Ring Library*.  
    https://doc.dpdk.org/guides/prog_guide/ring_lib.html

---

## 33. 一句话定义

> RFC-0002 把庭内核的“连廊优先”落成工程 ABI：Root Court 只通过 capability invocation 授权和治理，Court 之间只通过 typed Corridor 传输和协作，名字负责发现，能力负责授权，连廊负责数据，观测负责治理。

