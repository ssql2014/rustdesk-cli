# Evas E200 "Epoch" Chip — Comprehensive Learning Report

**Date**: 2026-03-15
**Sources**: 异构编程手册-0302.pdf, visa-op.pdf, 高性能算子实现指南.pdf, and all source files in
`workspace/evmme/test/hgss/v1/vllm_ops/torch_ops/`

---

## Table of Contents

1. [Chip Architecture](#1-chip-architecture)
2. [AC Language and Kernel Programming Model](#2-ac-language-and-kernel-programming-model)
3. [VISA Operator API](#3-visa-operator-api)
4. [Runtime and Memory Management APIs](#4-runtime-and-memory-management-apis)
5. [Synchronization Mechanisms](#5-synchronization-mechanisms)
6. [Transfer Engine API](#6-transfer-engine-api)
7. [Performance Optimization Techniques](#7-performance-optimization-techniques)
8. [Existing Operator Implementations](#8-existing-operator-implementations)
9. [Key Constants and Constraints](#9-key-constants-and-constraints)

---

## 1. Chip Architecture

### 1.1 Top-Level Structure

The Evas E200 "Epoch" chip is a custom AI accelerator with the following top-level composition:

| Component | Count | Details |
|-----------|-------|---------|
| DDR channels | 24 | 2 GB each → **48 GB total** |
| Clusters | 8 | Each with 4 cores + 9 MB L2 + 1 TEC |
| TES groups | 2 | Cross-die / RDMA / UCIE transfer |
| Dies per SoC | 2 | Each die holds 4 clusters |

### 1.2 Cluster and Core Architecture

```
Chip
├── Die 0
│   ├── Cluster 0  (9 MB L2, TEC0+TEC1)
│   │   ├── Core 0 (1.5 MB MM + 0.25 MB AM, VE, ME, TE)
│   │   ├── Core 1
│   │   ├── Core 2
│   │   └── Core 3
│   ├── Cluster 1
│   ├── Cluster 2
│   └── Cluster 3
└── Die 1
    ├── Cluster 4
    ├── Cluster 5
    ├── Cluster 6
    └── Cluster 7
```

**Per-Core Compute Engines:**

| Engine | Role |
|--------|------|
| ME (Matrix Engine) | Dense matrix multiplication; operates on MM/AM |
| VE / VPU (Vector Processing Unit) | Element-wise math, activations, normalization |
| TE (Transfer Engine) | Intra-cluster data movement (L2↔MM, AM↔L2, etc.) |
| MCU (Micro-Control Unit) | Instruction dispatch, scheduling |

### 1.3 Memory Hierarchy

| Level | Per-unit | Total (chip) | Scope | Access Speed |
|-------|----------|--------------|-------|--------------|
| MM (Matrix Memory, L1) | 1.5 MB / core | ~48 MB | Private per core | Fastest |
| AM (Accumulator Memory, L1) | 0.25 MB / core | ~8 MB | Private per core | Fastest |
| L2 (Shared Cache) | 9 MB / cluster | 72 MB | Shared per cluster | Medium |
| DDR (Global Memory) | 2 GB / channel | 48 GB | All cores (via TEC/TES) | Slowest |

**L2 Physical Organization:**
- 3 slices × 3 MB each
- 8 bank groups × 1024-bit per bank group per slice

**MM Physical Organization:**
- 6 slices × 256 KB each
- 4 bank groups × 512-bit per bank group per slice

**AM Physical Organization:**
- 1 slice × 256 KB
- 8 bank groups × 512-bit

### 1.4 Transfer Engines

| Engine | Bandwidth | Channels | Direction | Scope |
|--------|-----------|----------|-----------|-------|
| TE | 1024 bit/cycle | 3 | L2→MM, AM↔{MM,L2,DDR}, MM↔{MM,L2} | Intra-core |
| TEC0 | 1024 bit/cycle | 2 | DDR ↔ L2 | Intra-cluster (preferred for DDR) |
| TEC1 | 1024 bit/cycle | 1 | DDR↔L2, L2→L2 (line_copy only) | Intra-cluster |
| TES0/TES1 | 1024 bit/cycle | 1 each | AXI ↔ AXI (L2/DDR ↔ L2/DDR) | Cross-die / RDMA / UCIE |

---

## 2. AC Language and Kernel Programming Model

AC (Accelerator C) is Evas's C++17-compatible heterogeneous programming language. The compiler is `evcc`.

### 2.1 Kernel Declaration and Launch

```cpp
// Declare a kernel
__global__ void my_kernel(int *data, int size) {
    // Executes on device; CoreID/ClusterID available
    printf("core %d/%d\n", CoreID, CoreNum);
}

// Launch from host
my_kernel<<<N_CLUSTERS, N_CORES, stream, affinity, flags>>>(data, size);
```

**Launch parameters:**

| Parameter | Range | Default | Notes |
|-----------|-------|---------|-------|
| N_CLUSTERS | 1–8 | — | Number of clusters involved |
| N_CORES | 1–4 | — | Cores per cluster |
| stream | evStream_t | 0 | Implicit (0) or explicit stream |
| affinity | uint64_t | evAffinityMapDefault | Bitmask binding to specific die/cluster |
| flags | uint | 0 | Runtime flags |

**Kernel constraints:** no return value, no recursion in `__global__`, no `long`/`double` parameters.

### 2.2 Function Qualifiers

| Qualifier | Executes On | Called From | Notes |
|-----------|-------------|-------------|-------|
| `__host__` (default) | Host CPU | Host | Can use STL/libc |
| `__global__` | Device (all cores) | Host | Kernel entry point; void return |
| `__device__` | Device | Device | Can be recursive |
| `__host__ __device__` | Both | Either | Can only call other dual-side functions |

### 2.3 Memory Qualifiers for Variables

| Qualifier | Backed By | Capacity Limit | Scope |
|-----------|-----------|----------------|-------|
| (none) / `__device__` | DDR / global | 48 GB device | Chip-wide |
| `__shared__` | L2 | 9 MB per cluster | Cluster-wide |
| `__local__` | MM (L1) | 1.5 MB per core | Per core |

### 2.4 Built-in Core/Cluster Identifiers

```cpp
CoreID        // Logical core index within the kernel launch
CoreNum       // Total core count in this kernel launch
ClusterID     // Logical cluster index
ClusterNum    // Total cluster count
DieID         // Logical die index (0 or 1)
PhyCoreID     // Physical core index
PhyClusterID  // Physical cluster index
PhyDieID      // Physical die index
```

### 2.5 Memory Address Space Macros

All VISA intrinsics require typed (scoped) pointer wrappers:

```cpp
am_ptr(addr)   // Points into AM space
mm_ptr(addr)   // Points into MM space
l2_ptr(addr)   // Points into L2 space
ddr_ptr(addr)  // Points into DDR space
void_ptr()     // Used when an optional output is not needed
```

### 2.6 Separate Compilation

Device and host code can be compiled separately:

```bash
# Compile kernel-only to ELF
evcc --ac-device-only -emit-device-elf -x ac device.cc -o kernel.elf

# Load and launch at runtime
evModuleLoadData(&module, elf_buf, elf_size);
evLaunchKernel(&module, "kernel_name", NULL);
```

---

## 3. VISA Operator API

The `visa::` namespace provides hardware intrinsics targeting the VPU (VE), Matrix Engine (ME), and Transfer Engine (TE). All operators are called from device code (MCU context).

### 3.1 Naming Conventions

Function names follow the pattern:
```
<op>_<shape_variant>[_<modifier>]
```

**Shape variants:**
- `_mm` — matrix × matrix (both inputs 2D)
- `_mv_dimh` — matrix × vector (vector broadcast along rows)
- `_mv_dimw` — matrix × vector (vector broadcast along columns)
- `_mf` — matrix × scalar
- `_m` — single matrix input
- `_v` — vector input
- `_ms` — matrix-to-scalar (full reduction)

**Common modifiers:**
- `_fast` — reduced precision, higher throughput
- `_withFactor` — includes a multiplicative scale factor argument

### 3.2 Common Template Parameters

```cpp
template <core CORE = CORE0,    // VPU core, default CORE0
          typename CT = float32_t,  // Computation type (accumulation precision)
          typename T0,          // Output pointer type
          typename T1,          // Input 1 pointer type
          typename T2>          // Input 2 pointer type (if applicable)
MCU_FN void op(T0 pOut0, T1 pIn1, T2 pIn2,
               uint16_t M, uint16_t N,
               uint16_t SOUT0 = N,  // Output row stride
               uint16_t SIN1 = N,   // Input 1 row stride
               uint16_t SIN2 = N);  // Input 2 row stride
```

- `ETYPE<T0>` — helper to extract element scalar type from a scoped_ptr type
- `pBuffer` — temporary scratch buffer (size requirements vary per operator)

### 3.3 Supported Data Types

**Homogeneous (same type for all arguments):**
`int8_t`, `uint8_t`, `int16_t`, `uint16_t`, `int32_t`, `uint32_t`,
`float_e5m2_t`, `float_e4m3_t`, `float16_t`, `bfloat16_t`, `float32_t`

**Mixed-type combinations (examples):**
```
[float32_t input → float16_t output]
[float32_t input → bfloat16_t output]
[float16_t input → float_e5m2_t output]
[float16_t input → float32_t accumulation]
```

### 3.4 Arithmetic Operators

| Category | Operators |
|----------|-----------|
| Binary | `add`, `sub`, `mul`, `div`, `mod`, `floordiv`, `floormod`, `truncdiv` |
| Fused | `add_relu`, `sadd` (saturating), `ssub`, `rsub` |
| MAC | `macc`, `msac` (multiply-subtract-accumulate) |
| Bitwise | `bitwise_and`, `bitwise_or`, `bitwise_xor`, `bitwise_not` |
| Logical | `logical_and`, `logical_or`, `logical_xor`, `logical_not` |
| Shift | `lshift`, `rshift`, `rshifta` |

### 3.5 Math Functions (Unary)

| Category | Operators |
|----------|-----------|
| Root/Power | `sqrt`, `sqrt_fast`, `cbrt`, `rsqrt`, `pow`, `recip` |
| Exponential | `exp`, `exp2`, `exp2_fast` |
| Logarithm | `ln`, `ln_fast`, `log2`, `log2_fast`, `log10`, `log10_fast` |
| Trigonometric | `sin`, `cos`, `tan`, `arcsin`, `arccos`, `arctan`, `atan2`, `sinc` |
| Rounding | `fceil`, `ffloor`, `fround`, `fround_toeven`, `fround_tozero` |
| Special | `erf`, `erfinv`, `lgamma`, `xlogy`, `fabs`, `fneg`, `frexp` |

### 3.6 Activation Functions

```cpp
// All share the same signature pattern:
template <core CORE = CORE0, typename CT = float32_t, typename T0, typename T1>
MCU_FN void relu_m(T0 pOut0, T1 pIn1, uint16_t M, uint16_t N);

// Available activations:
relu, relu6, leakyrelu, selu, celu       // ReLU family
sigmoid, hardsigmoid, hardswish          // Sigmoid family
tanh                                      // Hyperbolic
gelu, glu, mish, silu                    // Gated / special
softmax, logsoftmax, softplus, logsigmoid
```

### 3.7 Normalization Operators

```cpp
// RMSNorm (key for LLM inference):
MCU_FN void rmsnorm_m(T0 pOut0, T1 pIn1, uint16_t M, uint16_t N, float32_t eps = 1e-6f);
MCU_FN void rmsnorm_m_withFactor(T0 out, T1 in0, T2 factor, uint16_t M, uint16_t N, float32_t eps);

// LayerNorm:
MCU_FN void layernorm_m(T0 pOut0, T3 pIn3, T4 pIn4, T5 pIn5,
                        uint16_t N, uint16_t C, uint16_t H, uint16_t W,
                        int32_t axis, float32_t eps = 1e-7f);

// BatchNorm (training + inference):
MCU_FN void batchnorm_m(T0 pOut0, T1 pOut1, T2 pOut2, T3 pOut3, T4 pOut4,
                        T5 pIn5, T3 pIn6, T4 pIn7, T6 pIn8, T7 pIn9,
                        uint16_t N, uint16_t H, uint16_t W,
                        float32_t eps = 1e-5f, float32_t momentum = 0.9f);
```

### 3.8 Reduction Operators

| Class | Variants |
|-------|----------|
| Accumulation | `reduce_acc_ms`, `reduce_acc_mv_dimh`, `reduce_acc_mv_dimw`, `_fast` variants |
| Product | `reduce_mul_ms`, `reduce_mul_mv_dimh` |
| Max / Min | `reduce_max_ms/mv_dimh/mv_dimw`, `reduce_min_*` |
| Mean | `reduce_mean_ms/mv_dimh/mv_dimw` |
| Variance | `reduce_variance_ms/mv_dimh/mv_dimw` |
| Median | `reduce_median_mv_dimw` (returns value + index) |
| Lp-norm | `reducelp_m` |
| Log-sum-exp | `reducelogsum_mv_dimh`, `reducelogsum_mv_dimw` |
| MAC-based | `reduce_macc_fast_mms/mmv_dimh/mmv_dimw/mvv_dimh/mvv_dimw` |

**Temporary buffer sizing (reduce_acc_ms example):**
```
N < 64        → 0 bytes
64 ≤ N < 256  → 2^(floor(log2(N))-1) elements
256 ≤ N < 4096 → 2^(floor(log2(N))-2) elements
N ≥ 4096      → 2048 elements
```

### 3.9 Comparison and Selection

```cpp
less_equal_mm, not_equal_mm              // Element-wise comparison
where_mm, where_mf                       // Ternary: cond ? T : F
where_idx_m, where_idx_v                 // Return indices of true elements
topk_val_idx_m, topk_gating_val_idx_m   // Top-K with index tracking
sort_val_idx_m, sort_val_m              // Row-wise sort
searchsorted_v                           // Binary search (insert position)
top_p_m, top_p_v                         // Probability threshold filter
```

### 3.10 RoPE Operator

```cpp
MCU_FN void apply_rope_SBN(T0 pOut0, T1 pIn1, T2 pIn2,
                            uint16_t M, uint16_t N, ...);
MCU_FN void init_rope(...);
```

### 3.11 Pooling Operators

```cpp
MCU_FN void avg_pooling_m(T0 pOut0, T1 pIn1, uint16_t D, uint16_t M, ...);
MCU_FN void max_pooling_m(T0 pOut0, T1 pOut1, T2 pIn2, uint16_t D, uint16_t M, ...);
MCU_FN void globalavgpool(T0 out, T1 in0, int32_t H_G, int32_t W_G, int32_t C_G, ...);
MCU_FN void lppool2d(...);
MCU_FN void lppool3d(...);
```

### 3.12 Data Layout Operators (VPU side)

`broadcast`, `transpose`, `permute`, `slice`, `gather`, `scatter`, `pad`, `flip`, `tril`, `rope`, `maskedfill`, `unique`, `bincount`

### 3.13 Cumulative Operators

```cpp
static MCU_FN void cumsum(T0 pOut0, T1 pIn1, T2 pBuffer,
                          uint16_t M, uint16_t N,
                          int16_t axis = 1,
                          bool exclusive = false,
                          bool reverse = false);
// Also: cummax, cummin (return value AND index)
```

### 3.14 Remote VPU Call (RVC)

RVC allows the MCU to off-load custom RISC-V vector code to the VPU:

```cpp
// Create and dispatch an RVC
uint32_t base_reg = create_rvc<CORE0, M, N, C, SOUT, SIN0, SIN1>(
    cluster_id, core_id, (uint64_t)&vpu_func,
    VPU_RVC_PRIORITY_L, dst, src1, src2, scalars);
invoke_rvc(base_reg, VPU_START);
wait_rvc(base_reg);
```

The VPU function uses standard RISC-V vector intrinsics (`__riscv_vle32_v_i32m1`, `__riscv_vadd_vv_i32m1`, etc.).

---

## 4. Runtime and Memory Management APIs

### 4.1 Device Management

```cpp
evError_t evSetDevice(int device);
evError_t evGetDevice(int* device);
evError_t evGetDeviceCount(int* count);
evError_t evGetDeviceName(const char **name, int device);
evError_t evGetDeviceAttribute(int* value, evDeviceAttr attr, int device);
evError_t evDeviceSynchronize(void);  // Wait for ALL device work
```

### 4.2 Stream Management

```cpp
evError_t evStreamCreate(evStream_t* pStream,
                         uint64_t affinitymap = evAffinityMapDefault);
evError_t evStreamDestroy(evStream_t stream);
evError_t evStreamSynchronize(evStream_t stream);
evError_t evStreamClean(evStream_t stream);    // Cancel all queued ops
evError_t evStreamWaitEvent(evStream_t stream, evEvent_t event);
```

**Implicit stream (0)**: accepts both sync and async requests, Sequential by default.
**Explicit streams**: async only; independent streams can execute in parallel.

### 4.3 Event Management

```cpp
evError_t evEventCreate(evEvent_t *pEvent);
evError_t evEventDestroy(evEvent_t event);
evError_t evEventRecord(evEvent_t event, evStream_t stream = 0);
evError_t evEventSynchronize(evEvent_t event);  // Block until event fires
evError_t evEventElapsedTime(float *ms, evEvent_t start, evEvent_t end);
```

### 4.4 Memory Allocation

```cpp
// Synchronous host-accessible
evError_t evMalloc(void** devPtr, size_t size,
                   uint64_t affinitymap = evAffinityMapDefault);
evError_t evFree(void* devPtr);

// Async allocation from default pool
evError_t evMallocAsync(void **devPtr, size_t size, evStream_t stream = 0,
                        uint64_t affinitymap = evAffinityMapDefault);
evError_t evFreeAsync(void* devPtr, evStream_t stream = 0);

// Async from custom pool
evError_t evMallocFromPoolAsync(void **devPtr, size_t size, evMemPool_t memPool,
                                evStream_t stream = 0,
                                uint64_t affinitymap = evAffinityMapDefault);

// Host-pinned (zero-copy)
evError_t evMallocHost(void** ptr, size_t size);
evError_t evFreeHost(void* ptr);
evError_t evHostRegister(void* ptr, size_t size);   // Pin existing allocation
evError_t evHostUnregister(void* ptr);
```

### 4.5 Memory Pool

For performance-critical workloads, use a pre-allocated continuous pool:

```cpp
evMemPoolProps poolProps = {
    .preAllocContinuousMode = true,
    .preAllocDie0MaxSize    = 200 * 1024 * 1024,  // 200 MB on die 0
    .preAllocDie1MaxSize    = 200 * 1024 * 1024,  // 200 MB on die 1
    .maxSize                = 0,
};
evMemPoolCreate(&memPool, &poolProps);
// ... use evMallocFromPoolAsync ...
evMemPoolDestroy(memPool);
```

Pool attribute control:
```cpp
evError_t evMemPoolSetAttribute(evMemPool_t memPool, evMemPoolAttr attr, void* value);
evError_t evMemPoolGetAttribute(evMemPool_t memPool, evMemPoolAttr attr, void* value);
evError_t evMemPoolGetBaseAddr(evMemPool_t memPool, void** die0, void** die1);
evError_t evMemPoolTrimTo(evMemPool_t memPool, size_t minBytesToKeep);
```

### 4.6 Host ↔ Device Memory Copy

```cpp
evError_t evMemcpy(void* dst, const void* src, size_t size, evMemcpyKind kind);
evError_t evMemcpyAsync(void* dst, const void* src, size_t size,
                        evMemcpyKind kind, evStream_t stream = 0);
```

Direction constants: `evMemcpyHostToDevice`, `evMemcpyDeviceToHost`, `evMemcpyDeviceToDevice`

### 4.7 Device-Side Memory Utilities

```cpp
// From device code:
device void *memset(void *ptr, int data, unsigned long length);
device int   memcmp(const void *s1, const void *s2, size_t n);
device uint64_t memcpy_general(uint64_t dst, uint64_t src, uint32_t size,
                               memcpy_dir_t dir);
device unsigned long clock64(void);          // High-resolution timer
device void __nanosleep(unsigned long ns);   // Precision sleep
device uint8_t cluster_map(void);            // Logical→physical cluster ID
device int dump_memory(memory_kind_t kind, unsigned long start,
                       unsigned long size, const char *filename);
```

### 4.8 Error Handling

```cpp
evError_t evGetLastError(void);
const char* evGetErrorName(evError_t error);
const char* evGetErrorString(evError_t error);
```

Key error codes: `evSuccess(0)`, `evErrorInvalidValue(1)`, `evErrorDeviceMemoryAllocation(4)`, `evErrorInvalidKernel(7)`, `evErrorDeviceImageException(15)`

---

## 5. Synchronization Mechanisms

### 5.1 Device-Side Sync Functions

```cpp
device void sync(void);        // Barrier for all cores in cluster
device void gsync(void);       // Global barrier for all cores in launch
device void sync_tec(void);    // Wait for TEC module to finish
device void sync_te(void);     // Wait for TE module to finish
```

### 5.2 FENCE Instructions

Hardware-level ordering between engines on a single core:

| Instruction | Effect |
|-------------|--------|
| `FENCE_ALL` | Drain all engines (ME, VE, TE) |
| `FENCE_ME` | Wait for Matrix Engine |
| `FENCE_VE` | Wait for Vector Engine |
| `FENCE_TE` | Wait for Transfer Engine (intra-cluster) |
| `FENCE_TEC0` | Wait for TEC0 (DDR↔L2 engine 0) |

### 5.3 Cluster-Level Sync

```cpp
sync();      // Wait for all 4 cores in this cluster
sync_te();   // Cluster-wide TE barrier
sync_tec();  // Cluster-wide TEC barrier
```

### 5.4 Die and Chip-Level Sync

```cpp
die_layer_sync();   // Synchronize all clusters within one die
chip_layer_sync();  // Synchronize all clusters across both dies (full chip)
```

### 5.5 Notify Mechanism (Fine-Grained Async)

Producer-consumer event notification for overlapping transfers and computation:

**Sender side:**
```cpp
// Update actTrigger_number via TE/TEC descriptor
template <ENGINE send_engine, ENGINE receive_engine, ...>
static MCU_FN void mroute_set(uint32_t notify_id);
```

**Receiver side:**
```cpp
template <ENGINE receive_engine>
MCU_FN void mcu_notify_engine(int dsc, int die_id=0, int cluster_id=0, int core_id=0);
```

The receiver blocks until `actTrigger_number ≥ trigger_number`. This eliminates idle spinning and allows the MCU to schedule other work. `notify_id` range: **0–41** (42 total descriptors).

### 5.6 Engine Trigger Initialization

```cpp
template <ENGINE E>
static MCU_FN void trigger_init();   // Initialize trigger registers before use

MCU_FN void wait_for_mcu();          // VPU function: wait for MCU signal
```

---

## 6. Transfer Engine API

### 6.1 `line_copy` — Bulk Data Movement

```cpp
template <ENGINE E,
          uint64_t SRC_BUFFER_SIZE,
          int H, int W,
          SHARED_REDUCE_MODE mode = NO_REDUCE,
          BROADCAST_DIR bcast = RSV_CLUSTER,
          bool IS_CROSS_CLUSTER = false>
MCU_FN uint64_t line_copy(T0 src, T1 dst, uint32_t *index_ram_addr,
                           Notify notify = {NOTIFY_NONE, 0},
                           ddr_qos qos = CHN_PRIORITY_5);
```

- **Reduce modes**: `NO_REDUCE` (overwrite), `REDUCE_ACC` (+=), `REDUCE_MIN`, `REDUCE_MAX`
- **Broadcast**: `COMBINE_CLUSTER(Cluster_dir, UCIE_Cluster_dir)` for multi-cluster fan-out
- **IS_CROSS_CLUSTER**: set `true` for cross-die operations via TES

### 6.2 `gather` and `scatter`

```cpp
MCU_FN uint64_t gather(T0 src, T1 dst, uint32_t *index_ram_addr,
                        uint64_t SRC_BUFFER_SIZE, SHAPE_TUPLE shape, ...);

MCU_FN uint64_t scatter(T0 src, T1 dst, uint32_t *index_ram_addr,
                         uint64_t DST_BUFFER_SIZE, SHAPE_TUPLE shape, ...);

// Ping-pong variant for double-buffering patterns:
MCU_FN void gather_scatter_pingpong(SRC_DTYPE *src, DST_DTYPE *dst,
                                    uint32_t *index_cache_addr, ...);
```

**Key constraints for gather/scatter:**
- Must call `copy_indexram()` to load indices before issuing the operation
- Index values are `uint32_t` byte offsets relative to the buffer base
- Base address offsets: DDR → add `DNOC_DDR0_BASE` (`0x1000000000UL`); L2 → add `EVAMIND_L2_BASE` (`0x42000000`); MM → add `EVAMIND_MM_BASE` (`0x200000`)
- Index buffer: 0x2000 bytes total (2 × 0x1000 banks), max **2048 indices**
- Cannot issue to the same engine until current index_ram transfer is complete
- Different engines can run gather/scatter in parallel
- **TEC preferred over TES** for performance

### 6.3 Other TE Operations

```cpp
MCU_FN uint64_t transpose(T0 src, T1 dst, uint16_t H, uint16_t W, ...);
MCU_FN uint64_t permute(T0 src, T1 dst, uint32_t *index_ram_addr, ...);
MCU_FN uint64_t slice(T0 src, T1 dst, uint64_t offset, uint64_t slice_size, ...);
MCU_FN uint64_t pad(T0 src, T1 dst, uint32_t *index_ram_addr, ...);
MCU_FN uint64_t broadcast(T0 src, T1 dst, uint32_t *index_ram_addr, ...);
MCU_FN uint64_t constant_fill(T0 dst, uint32_t value, uint64_t size, ...);
```

### 6.4 `memcpy_dir_t` Direction Enum

```
DIR_L2TOMM       DIR_DDRTOL2_0 (TEC0)    DIR_AMTOL2
DIR_MMTODDR      DIR_L2TODDR_0 (TEC0)    DIR_AMTOMM
DIR_DDRTOMM      DIR_DDRTOL2_1 (TEC1)    DIR_AMTODDR
DIR_MMTOL2       DIR_L2TODDR_1 (TEC1)    DIR_MMTOMM
DIR_DDRTOL2      DIR_L2TOL2 (TEC1)       DIR_AXITOAXI
```

### 6.5 System Configuration Functions

```cpp
set_l2_bank_mode();          // Configure L2 bank interleaving
set_encode_invert_mode();    // Configure encode/invert mode
set_ucie_bufferable();       // Enable early response on UCIE links
set_bcs_cluster();           // Assign clusters to broadcast station
set_ucie_bcs();              // Configure UCIE broadcast station
trigger_clean(bitmap);       // Reset engine descriptors
```

---

## 7. Performance Optimization Techniques

### 7.1 Bank Conflict Avoidance

**Bank conflict conditions**: two or more simultaneous accesses to the **same bank group and same slice**.

**ME weight-loading conflict cases:**

| Weight dtype | Rows/cycle | Risky stride (elements) | Safe stride |
|-------------|------------|--------------------------|-------------|
| float16 | 2 | 128 (= 4 × 512-bit bank groups) | 64 |
| int8 | 4 | 256 (all 4 rows hit same bank group) | 64 |

**Mitigation strategies:**
- Distribute FM, Weight, Bias, Output to **different MM slices**
- Store intermediate VE results in a slice separate from ME inputs
- Use `static_assert` or address arithmetic to verify slice placement at compile time

### 7.2 Double-Buffering (Software Pipelining)

The canonical technique for hiding DDR↔L2↔MM transfer latency:

```
Buffer A, Buffer B (each holds one tile of data):

Iteration 0:  Load tile-0 → Buffer A           [TE/TEC busy]
Iteration 1:  Compute tile-0 (A)  |  Load tile-1 → Buffer B  [ME+TEC overlap]
Iteration 2:  Compute tile-1 (B)  |  Load tile-2 → Buffer A
              ...
```

- Achieves ~50% tail overhead reduction vs. non-pipelined version (measured in guide)
- ME/VE **auto-detect data dependencies** on L1 addresses — no manual FENCE needed when sequential ops touch the same address range
- Manual sync still needed across (ME → VE output going to another core via TE)

### 7.3 Tiling and Iteration Space

**Tiling factors** (example: GEMM on 512×4096×4096 M×K×N):
- M-tile: 128 per core (4-core parallelism inside cluster)
- K-tile: 512 chunks (determines inner-loop L2 residency)
- N-tile: 128 or 256 chunks (determines outer-loop cluster partition)

**Memory residency limits per core (guideline):**
```
FM in MM:     ≤ 512 KB
Weight in MM: ≤ 512 KB
Bias in MM:   ≤ 8 KB
Output in AM: ≤ 128 KB  (PAM ≤ 256 KB)
Total MM:     ≤ 1.5 MB
```

### 7.4 ME Throughput Modes

| Mode | FM dtype | Weight dtype | Accum dtype | Tile |
|------|----------|-------------|-------------|------|
| VMC_F16TOF32_M1K64N64 | float16 | float16 | float32 | 1×64×64 |
| VMC_S8S4TOS32_M1K128N128 | int8 | int4 | int32 | 1×128×128 |
| (others) | BF16, FP8 | BF16, FP8 | FP32 | varies |

**ME input shape recommendations:**
- M ≥ 32 (amortizes weight-loading overhead)
- N: must be multiple of 32 (or 128 for S8S4 mode)
- K: no strict alignment requirement

### 7.5 VE (VPU) Efficiency

- Minimum tile for full utilization: **512 elements** (FP16), **1024 elements** (INT8)
- Use full LMUL=8 RISC-V vector loads/stores for 1024-bit throughput
- TE load/store from MM has ~7-cycle latency; avoid back-to-back LD→compute on same address
- When stride prevents full-width access, insert an explicit TE copy to make data contiguous

### 7.6 TEC Bandwidth Optimization

- Prefetch next tile into a staging L2 buffer while current tile is being processed
- Use larger burst sizes for higher DDR channel utilization
- Partition weights across clusters (each cluster owns its slice) rather than broadcasting
- TEC0 (2 channels) has 2× the DDR bandwidth of TEC1 (1 channel); prefer TEC0 for heavy DDR traffic

### 7.7 Notify vs. Blocking Sync

| Mechanism | Use Case | Overhead |
|-----------|----------|----------|
| `sync()` | Cluster barrier when all cores must agree | High (stall all cores) |
| `FENCE_ME/VE/TE` | Engine ordering within one core | Low (waits for one engine) |
| Notify | Producer-consumer pipeline overlap | Very low (MCU polls register) |

**Prefer Notify** whenever the producer and consumer can overlap; use `sync()` only when a full cluster rendezvous is required.

---

## 8. Existing Operator Implementations

All operators are in `workspace/evmme/test/hgss/v1/vllm_ops/torch_ops/`. Each operator has a `.ac` kernel file and a `_host.cpp` launcher file, plus a `CMakeLists.txt`.

### 8.1 Initialization — `vllm_fusedop_init.ac`

Run once before any inference. Configures device-wide state:
- Sets L2 bank mode
- Assigns notify IDs per engine type
- Sets up TEC/TE trigger chains
- Configures broadcast cluster routing (`set_bcs_cluster`, `set_ucie_bcs`)

### 8.2 Linear / GEMM — `linear_T/`

**Key files:** `linear_T.ac`, `linear_T_host.cpp`

Implements `Y = X @ W^T` (weights are stored transposed for ME access patterns).

**Kernel structure:**
- Launch: `<<<CLUSTER_NUM, CORE_NUM, stream>>>`
- Each cluster handles a tile of output rows
- TE loads input feature map (FM) from DDR→L2→MM
- TEC prefetches weight tiles cluster-by-cluster
- ME executes `matmul` in double-buffered K-loops
- VE applies optional bias add
- TE writes output back to DDR

**Launch interface:**
```cpp
void launch_linear_t_die_passthrough(
    void* output_d0, void* output_d1,
    void* input_d0, void* input_d1,
    void* weight_d0, void* weight_d1,
    int* cu_seqlens_d0, int* cu_seqlens_d1,
    int dtype, int batch_len, int in_dim, int out_dim);
```

### 8.3 GQA Attention — `attention/`

**Key files:** `attention_kernel.ac` (2272 lines), `MQA_decode_die.ac` (1627 lines), `attention_host.cpp`

Implements grouped-query attention (paged KV cache) for decode and prefill phases.

**Current hardware constraints (as of this codebase):**
- `q_head_cnt` must equal **8** (asserted at line 188–189 of attention_kernel.ac and line 171 of MQA_decode_die.ac)
- `kv_head_cnt_die` must equal **1** (line 1998 of attention_kernel.ac — MQA only per die)
- `kv_head_cnt_soc` must equal **2** (line 2061 of attention_kernel.ac)
- KV gather owners are hardcoded to `CLUSTER_ID==0` and `CLUSTER_ID==3`
- `set_kv_block_idx` stride ignores `kv_head_cnt` (bug: always uses `block_size * head_dim * bytes`)

**Kernel flow (decode path):**
1. `set_kv_block_idx` — builds DDR gather index tables for paged KV blocks
2. Cluster scatter: each cluster gathers its assigned Q heads
3. `MQA_single` (short KV) or `MQA_batch` (long KV, k_len ≥ 512) — core attention
4. Softmax merge across KVs (split-KV reduction)
5. `MQA_prefill_die` — prefill attention for long prompt sequences

**Paged KV cache layout:**
```
KV cache: [num_blocks, block_size, kv_head_cnt, head_dim]
kv_block_idx: maps logical block position → physical block index
```

**Launch interface (target):**
```cpp
void launch_attention_scheduler_die_passthrough(
    void* output_combine_d0, void* output_combine_d1,
    void* Q_combine_d0, void* Q_combine_d1,
    void* K_combine_d0, void* K_combine_d1,
    void* V_combine_d0, void* V_combine_d1,
    void* kv_block_idx_d0, void* kv_block_idx_d1,
    int* cu_seqlens_q_d0, int* cu_seqlens_q_d1,
    int* seq_used_kv_d0, int* seq_used_kv_d1,
    float softmax_scale, float softcap,
    int attention_type, int device_id, int dtype,
    int batch_len, int max_seqlen_q, int max_kv_block_cnt,
    int q_head_cnt, int kv_head_cnt, int head_dim,
    int block_size, bool casual = false);
```

**GQA generalization plan** is documented at `/home/evas/gqa_implementation_plan.md`.

### 8.4 MLP (SwiGLU) — `mlp/`

**Key files:** `mlp.ac`, `mlp_host.cpp`

Implements the Llama-style SwiGLU MLP block:
```
out = (x @ W_gate) ⊙ SiLU(x @ W_up) @ W_down
```

- Gate projection and up projection run as parallel GEMMs
- Element-wise SiLU applied to gate output
- Hadamard product and down projection follow
- Each cluster handles an output tile

### 8.5 RMSNorm — `rmsnorm/`

**Key files:** `rmsnorm.ac`, `rmsnorm_host.cpp`

Implements:
```
out = x / RMS(x) * γ
RMS(x) = sqrt(mean(x²) + ε)
```

**Bug fixed (committed 2026-03-14):** `launch_rmsnorm_die_passthrough` had wrong argument order and was missing `output_res_ddr0`. Fixed args array:
```cpp
void* args[] = {
    &out_ddr0, (void*)8, &output_res_ddr0, (void*)8,
    &fm_ddr0, (void*)8, &wt_ddr0, (void*)8,
    &in_res_ddr0, (void*)8, &seq_len0, (void*)8,
    &dim_size0, (void*)8, &dtype, (void*)8,
    &eps, (void*)8, NULL
};
```

### 8.6 RoPE — Fused into GEMM+Norm+RoPE

**Key files:** `gemm_rope/`, `gemm_norm_rope/`

Rotary Position Embedding is not a standalone kernel — it is fused into the Q/K projection GEMM for efficiency. The fused kernels apply RoPE inline on the output of the projection, saving a round-trip to DDR.

**gemm_norm_rope** additionally fuses an RMSNorm before the matmul.

### 8.7 KV Cache Reshape — `reshape_and_cache_flash/`

**Key files:** `reshape_and_cache_flash.ac`, `reshape_and_cache_flash_host.cpp`

Reshapes new KV pairs from the attention output layout into the paged KV cache layout:
```
Input:  [batch, seq_len, kv_head_cnt, head_dim]  (contiguous from projection)
Output: paged KV blocks: [block_idx, block_pos, kv_head_cnt, head_dim]
```

Uses `kv_block_idx` to scatter each token's KV pair into its assigned physical block.

### 8.8 Reference Python Pipeline — `/home/evas/`

A complete NumPy reference implementation of the Llama 3 pipeline exists for validation:

| File | Purpose |
|------|---------|
| `rmsnorm_op.py` | RMSNorm reference |
| `matmul_op.py` | Matrix multiply reference |
| `silu_op.py` | SiLU activation reference |
| `rope_op.py` | RoPE reference (with `precompute_rope_freqs`) |
| `softmax_op.py` | Softmax reference |
| `embedding_op.py` | Token embedding lookup |
| `swiglu_mlp_op.py` | SwiGLU MLP reference |
| `llama3_pipeline_op.py` | Full pipeline: Embed→32×(Attn+MLP)→Norm→LM head→Softmax |

Run the smoke test:
```bash
python3 /home/evas/llama3_pipeline_op.py
# Expected: "SUCCESS: Llama3 pipeline smoke test passed."
```

---

## 9. Key Constants and Constraints

### 9.1 Hardware Constants

```cpp
CLUSTER_NUM     = 8      // Clusters per chip
CORE_NUM        = 4      // Cores per cluster
DIE_NUM         = 2      // Dies per SoC
CLUSTER_PER_DIE = 4      // Clusters per die
L2_SIZE         = 9 MB   // L2 per cluster
MM_SIZE         = 1.5MB  // MM (L1) per core
AM_SIZE         = 0.25MB // AM (L1) per core
```

### 9.2 Runtime Limits

```
Max simultaneous gather/scatter indices: 2048
Notify ID range: 0–41 (42 total)
Device-side stack: 1 MB (fixed, in global memory)
Max kernel argument: no long/double types in __global__
head_dim: {64, 128} (attention kernel)
kv_head_cnt_soc: ≥ 2 and must be even (KV cache constraint, per op_interface.md)
block_size: must satisfy block_size * head_dim * bytes ≤ gather index limits
```

### 9.3 ME Alignment Requirements

| Data | Location | Alignment |
|------|----------|-----------|
| Feature maps (FM) | MM | Element bit-width |
| Weight matrices | MM | 64 bytes |
| Bias | MM | 64 bytes |
| Output | AM | 64 bytes |

### 9.4 VE Minimum Tile Sizes

| dtype | Minimum elements for full utilization |
|-------|--------------------------------------|
| float16 / bfloat16 | 512 |
| int8 | 1024 |
| float32 | 256 |

### 9.5 git Repository

| Item | Value |
|------|-------|
| Repo root | `/home/evas/workspace/evmme` |
| Remote | `ssh://git@172.22.42.22:2828/ae/evmme.git` |
| Working branch | `vllm_ops_agent` (local only — no upstream pushed) |
| Test dir | `test/hgss/v1/vllm_ops/` |

---

*End of report. Generated from PDFs: 异构编程手册-0302.pdf, visa-op.pdf, 高性能算子实现指南.pdf, and all `.ac` / `.cpp` sources in `torch_ops/`.*
