# Phase 1 算子开发设计文档 — GemmaRMSNorm / RMSNormGated

**文档状态**：设计阶段（Design Phase），待客户评审
**撰写日期**：2026-03-15
**负责人**：evas
**代码分支**：`vllm_ops_agent`
**参考文件**：
- `torch_ops/rmsnorm/rmsnorm_kernel.ac`
- `torch_ops/rmsnorm/rmsnorm_broadcast_kernel.ac`
- `torch_ops/rmsnorm/rmsnorm_host.cpp`
- `torch_ops/rmsnorm/rmsnorm_broadcast_host.cpp`
- `op_interface.md`
- `/home/evas/rmsnorm_phase1_analysis.md`

---

## 目录

1. [范围与目标](#1-范围与目标)
2. [两种版本说明](#2-两种版本说明)
3. [数学公式](#3-数学公式)
4. [内核与主机接口设计](#4-内核与主机接口设计)
5. [内存布局](#5-内存布局)
6. [Tile 流水线流程](#6-tile-流水线流程)
7. [现有代码问题](#7-现有代码问题)
8. [待客户确认的开放问题](#8-待客户确认的开放问题)

---

## 1. 范围与目标

### 1.1 算子列表

本阶段（Phase 1）新增两个 RMSNorm 变体算子：

| 算子名 | 说明 | 对应 vLLM 用途 |
|--------|------|---------------|
| **GemmaRMSNorm** | RMSNorm 的 Gemma 变体，权重加 1（`γ+1`） | Gemma / Gemma 2 / Gemma 3 模型归一化层 |
| **RMSNormGated** | 带门控激活的 RMSNorm，输出为归一化结果与门控值的逐元素乘积 | GLU-based MLP、RMSNorm+Gating 融合层 |

### 1.2 数据类型支持

| dtype 编码 | 数据类型 | 说明 |
|-----------|---------|------|
| `dtype = 0` | `float16` | IEEE 半精度浮点 |
| `dtype = 1` | `bfloat16` | Brain floating point |

> **注意**：现有 `rmsnorm_kernel.ac` 中 `dtype=1` 被错误映射到 `float32`（详见 §7.1）。新算子统一采用上表约定，与 `rmsnorm_broadcast_kernel.ac` 及 `op_interface.md` 保持一致。

### 1.3 硬件目标

- **芯片**：Evas E200 "Epoch"，双 Die，每 Die 4 Cluster × 4 Core
- **精度**：片上计算采用 `float32` 中间累加，输出转回 f16/bf16
- **内存层次**：DDR → L2（TEC0 搬运）→ MM（TE 搬运）→ VPU 计算 → MM 结果 → DDR 写回

---

## 2. 两种版本说明

### 2.1 版本 B — 独立计算（Independent）

**对应现有实现**：`rmsnorm_kernel.ac` / `rmsnorm_run_die_v2`

```
Die0 处理行 [0, ceil(M/2))   →  输出写入 out_ddr0
Die1 处理行 [ceil(M/2), M)   →  输出写入 out_ddr1
```

双 Die 完全解耦并行，各自持有本地那一半结果。适用于后续算子各自在本 Die 上消费结果的场景（如 Attention 前的 RMSNorm，每个 Die 已持有自己的 Q/K/V 分片）。

**特点**：
- 无跨 Die 数据传输，延迟低
- 双 Die 输出內容不同（各持有 M 的一半）
- 支持残差融合（`in_res` 可为 NULL）
- 支持残差输出（`output_res_ddr`）

### 2.2 版本 A — 广播 + Allgather（Broadcast + Allgather）

**对应现有实现**：`rmsnorm_broadcast_kernel.ac` / `rmsnorm_run_die_broadcast`

```
Die0 计算行 [0, m0)，完成后 tile 级广播到 Die1
Die1 计算行 [m0, M)，完成后 tile 级广播到 Die0
最终：双 Die 均持有完整 M×N 结果
```

**两条执行路径**：

| 路径 | 触发条件 | 说明 |
|------|---------|------|
| **退化路径**（Standalone） | `M < 64`（`SMALL_M_THRESHOLD`） | 双 Die 各自独立计算全量 M，不广播。输出不保证双 Die 一致 |
| **流水线路径**（Pipeline） | `M ≥ 64` | 各算一半 + tile 级 TEC0 DDR→DDR 异步广播，计算与广播重叠 |

**特点**：
- 广播完成后双 Die 均持有完整结果
- 适用于下游算子（如 GEMM）需要在两个 Die 上各自消费完整激活的场景
- 当前版本不含残差融合（新增算子如需要，须补充）
- 流水线广播：每完成一个 cluster tile 后立即异步 TEC0 广播，与下一 tile 计算并行

### 2.3 版本选择依据


```
需要双 Die 均持有完整 M×N → 选版本 A（broadcast）
下游算子各 Die 只用自己那一半 → 选版本 B（independent）
M < 64 的短序列 → 版本 A 自动退化为 standalone，等价于版本 B
```

---

## 3. 数学公式

### 3.1 标准 RMSNorm（已实现，参考基准）

$$\text{RMS}(x) = \sqrt{\frac{1}{N}\sum_{i=1}^{N} x_i^2 + \varepsilon}$$

$$\text{out}_j = \frac{x_j}{\text{RMS}(x)} \cdot \gamma_j, \quad j = 1, \ldots, N$$

- 输入：$x \in \mathbb{R}^{M \times N}$，权重 $\gamma \in \mathbb{R}^{N}$，$\varepsilon$（默认 1e-5）
- 归一化在行维度（N 轴）进行，每行独立

### 3.2 GemmaRMSNorm

$$\text{out}_j = \frac{x_j}{\text{RMS}(x)} \cdot (\gamma_j + 1), \quad j = 1, \ldots, N$$

与标准 RMSNorm 唯一的区别：权重从 $\gamma$ 变为 $\gamma + 1$。

**实现策略**：在权重加载到 MM 后，执行一次 VPU 加法：

```
load(wt_mm, wt_ddr, [1, N])      // 预加载权重
add_mf(wt_mm, wt_mm, 1.0f, 1, N) // wt_mm = γ + 1
// 后续 tile 循环不变
```

此操作每次 kernel 启动仅执行一次，分摊到所有 tile。

### 3.3 RMSNormGated

输入张量在 N 轴对半拆分为归一化分支和门控分支：

$$x_{\text{norm}}, x_{\text{gate}} = \text{split}(x, \text{dim}=-1) \quad \text{各形状} [M, N/2]$$

$$\text{out}_j = \frac{(x_{\text{norm}})_j}{\text{RMS}(x_{\text{norm}})} \cdot \gamma_j \cdot \sigma\!\left((x_{\text{gate}})_j\right), \quad j = 1, \ldots, N/2$$

其中 $\sigma$ 为门控激活函数，候选：

| 激活 | 公式 | VISA 算子 |
|------|------|----------|
| SiLU（推荐） | $x \cdot \sigma(x)$ | `silu_m` |
| Sigmoid | $1/(1+e^{-x})$ | `sigmoid_m` |

- 输入：`fm_ddr`（$[M, N/2]$）、`gate_ddr`（$[M, N/2]$）、`wt_ddr`（$[1, N/2]$）
- 输出：`out_ddr`（$[M, N/2]$）

> **待确认**：门控激活类型由客户指定（见 §8，问题 1）。

### 3.4 残差融合（版本 B 支持，适用于 GemmaRMSNorm）

当 `in_res` != NULL 时：

$$x_{\text{sum}} = x_{\text{fm}} + x_{\text{residual}}$$

$$\text{output-res} = x_{\text{sum}}$$

$$\text{out} = \text{RMSNorm}(x_{\text{sum}}, \gamma, \varepsilon)$$

---

## 4. 内核与主机接口设计

命名约定与现有算子一致，使用 `_passthrough` 后缀标识 Host 端启动器。

### 4.1 GemmaRMSNorm — 版本 B（独立）

#### 设备侧内核

```c
// 单 Die 内核
__GLOBAL__ void gemma_rmsnorm_run_die(
    void* out_ddr0,         // 归一化输出 [M, N]
    void* output_res_ddr0,  // 更新后的残差输出 [M, N]（= fm + in_res）
    void* fm_ddr0,          // 输入特征矩阵 [M, N]
    void* wt_ddr0,          // 权重 γ [1, N]
    void* in_res_ddr0,      // 残差输入 [M, N]；NULL 时不做残差加法
    int seq_len0,           // M
    int dim_size0,          // N
    int dtype,              // 0=float16, 1=bfloat16
    float eps);             // epsilon，默认 1e-5

// 双 Die 内核
__GLOBAL__ void gemma_rmsnorm_run_die_v2(
    void* out_ddr0,         void* output_res_ddr0,
    void* fm_ddr0,          void* wt_ddr0,          void* in_res_ddr0,
    void* out_ddr1,         void* output_res_ddr1,
    void* fm_ddr1,          void* wt_ddr1,          void* in_res_ddr1,
    int seq_len, int dim_size, int dtype, float eps);
```

#### Host 端启动器

```c
void launch_gemma_rmsnorm_die_passthrough(

    void* out_ddr0, void* output_res_ddr0, void* fm_ddr0, void* wt_ddr0,
    void* in_res_ddr0, float eps, int seq_len0, int dim_size0,
    int dtype, int device_id, int m_tile_size = 16);
```

参数说明与 `launch_add_rmsnorm_die_passthrough` 完全一致，仅内核名不同。

### 4.2 GemmaRMSNorm — 版本 A（广播+Allgather）

#### 设备侧内核

```c
__GLOBAL__ void gemma_rmsnorm_run_die_broadcast(
    void* out_ddr0, void* out_ddr1,   // 双 Die 输出（完整 M×N）
    void* fm_ddr0,  void* fm_ddr1,   // 双 Die 输入
    void* wt_ddr0,  void* wt_ddr1,   // 双 Die 权重
    int seq_len, int dim_size, int dtype, float eps);
```

#### Host 端启动器

```c
void launch_gemma_rmsnorm_broadcast_passthrough(
    void* out_ddr0, void* out_ddr1,
    void* fm_ddr0,  void* fm_ddr1,
    void* wt_ddr0,  void* wt_ddr1,
    int seq_len, int dim_size, int dtype, float eps, int device_id);
```

启动配置：`evConfigureCall(8, 4)`（8 Cluster × 4 Core，覆盖双 Die）。

### 4.3 RMSNormGated — 版本 B（独立）

#### 设备侧内核

```c
// 单 Die 内核
__GLOBAL__ void rmsnorm_gated_run_die(
    void* out_ddr0,     // 输出 [M, N/2]
    void* fm_ddr0,      // 归一化分支输入 [M, N/2]
    void* gate_ddr0,    // 门控分支输入 [M, N/2]
    void* wt_ddr0,      // 权重 [1, N/2]
    int seq_len0,       // M
    int dim_size0,      // N/2（已经是拆分后的半宽度）
    int dtype,          // 0=float16, 1=bfloat16
    float eps);         // epsilon

// 双 Die 内核
__GLOBAL__ void rmsnorm_gated_run_die_v2(
    void* out_ddr0, void* fm_ddr0, void* gate_ddr0, void* wt_ddr0,
    void* out_ddr1, void* fm_ddr1, void* gate_ddr1, void* wt_ddr1,
    int seq_len, int dim_size, int dtype, float eps);
```

#### Host 端启动器

```c
void launch_rmsnorm_gated_die_passthrough(
    void* out_ddr0, void* fm_ddr0, void* gate_ddr0, void* wt_ddr0,
    float eps, int seq_len0, int dim_size0,
    int dtype, int device_id, int m_tile_size = 16);
```

### 4.4 RMSNormGated — 版本 A（广播+Allgather）

#### 设备侧内核

```c
__GLOBAL__ void rmsnorm_gated_run_die_broadcast(
    void* out_ddr0,   void* out_ddr1,
    void* fm_ddr0,    void* fm_ddr1,
    void* gate_ddr0,  void* gate_ddr1,
    void* wt_ddr0,    void* wt_ddr1,
    int seq_len, int dim_size, int dtype, float eps);
```

#### Host 端启动器

```c
void launch_rmsnorm_gated_broadcast_passthrough(
    void* out_ddr0,   void* out_ddr1,
    void* fm_ddr0,    void* fm_ddr1,
    void* gate_ddr0,  void* gate_ddr1,
    void* wt_ddr0,    void* wt_ddr1,
    int seq_len, int dim_size, int dtype, float eps, int device_id);
```

启动配置：`evConfigureCall(8, 4)`。

### 4.5 新增文件规划

| 文件 | 说明 |
|------|------|
| `torch_ops/rmsnorm/gemma_rmsnorm_kernel.ac` | GemmaRMSNorm 设备侧内核（版本 A + B） |
| `torch_ops/rmsnorm/gemma_rmsnorm_host.cpp` | GemmaRMSNorm Host 端启动器 |
| `torch_ops/rmsnorm/gemma_rmsnorm.h` | GemmaRMSNorm 头文件 |
| `torch_ops/rmsnorm/rmsnorm_gated_kernel.ac` | RMSNormGated 设备侧内核（版本 A + B） |
| `torch_ops/rmsnorm/rmsnorm_gated_host.cpp` | RMSNormGated Host 端启动器 |
| `torch_ops/rmsnorm/rmsnorm_gated.h` | RMSNormGated 头文件 |


---

## 5. 内存布局

### 5.1 GemmaRMSNorm 内存布局

与现有 `rmsnorm_kernel.ac` 完全一致。唯一新增操作是权重加载后的一次 `add_mf`，不占用额外缓冲区。

**MM 地址分配（单 Core，m_tile=16，N=dim_size）：**

```
地址范围                  用途                      大小
0x000000 – 0x04FFFF      norm_mm_ping_in           m×N×2 = 16×N×2 字节
0x050000 – 0x09FFFF      norm_mm_pong_in           16×N×2 字节
0x0A0000 – 0x0EFFFF      norm_mm_pong_out          16×N×2 字节
0x0F0000 – 0x13FFFF      norm_mm_ping_out          16×N×2 字节
0x170000 – 0x170000+N×2  wt_mm                     1×N×2 字节
```

**L2 地址分配（Cluster 共享）：**

```
地址范围                  用途
0x000000 – 0x13FFFF      fm_l2_ping                4×m×N×2 字节（4 Core）
0x140000 – 0x27FFFF      fm_l2_pong
0x280000 – 0x3BFFFF      residual_l2_ping
0x3C0000 – 0x4FFFFF      residual_l2_pong
```

### 5.2 RMSNormGated 内存布局

相比标准 RMSNorm，需要额外的门控缓冲区（形状 `[m_tile, N/2]`）。

**MM 地址分配（m_tile=16，dim_size=N/2）：**

```
地址范围                  用途                      大小（N/2 = 2048 时）
0x000000 – 0x03FFFF      norm_mm_ping_in           16×2048×2 = 64 KB
0x040000 – 0x07FFFF      norm_mm_pong_in           64 KB
0x080000 – 0x0BFFFF      norm_mm_ping_out          64 KB
0x0C0000 – 0x0FFFFF      norm_mm_pong_out          64 KB
0x100000 – 0x13FFFF      gate_mm_ping              64 KB  ← 新增
0x140000 – 0x17FFFF      gate_mm_pong              64 KB  ← 新增
0x180000 – 0x180000+N    wt_mm                     2048×2 = 4 KB
```

**合计 MM 占用（N/2=2048，m_tile=16）：6×64KB + 4KB ≈ 388 KB，远低于 1.5 MB 限制。**

> 当 dim_size（N/2）增大时需重新核算。以 N/2=4096（总 N=8192）为例：
> 每块 = 16×4096×2 = 128 KB，合计 6×128 + 8 = 776 KB，仍在限制以内。

**L2 地址分配（新增 gate 缓冲）：**

```
地址范围                  用途
0x000000 – 0x0FFFFF      fm_l2_ping（归一化分支）  4×m×N/2×2 字节
0x100000 – 0x1FFFFF      fm_l2_pong
0x200000 – 0x2FFFFF      gate_l2_ping              4×m×N/2×2 字节  ← 新增
0x300000 – 0x3FFFFF      gate_l2_pong              ← 新增
```

---

## 6. Tile 流水线流程

### 6.1 GemmaRMSNorm — 版本 B（独立，基于现有 `rmsnorm` 模板）

与现有 `rmsnorm()` 流水线完全相同，增加 **权重预处理步骤 0**：

```
步骤 0（启动时执行一次）：
  TE: DDR → MM，加载权重 wt_ddr → wt_mm           [1, N]
  VPU add_mf: wt_mm = wt_mm + 1.0f               [1, N]  ← 新增

步骤 I（对每个 tile i，双缓冲 i%2）：
  CORE0 TEC0: DDR → L2    fm_ddr[offset] → fm_l2[i%2]    [m×4, N]
  若残差：CORE0 TEC0: DDR → L2 (REDUCE_ACC)  residual → fm_l2[i%2]
  TE: L2 → MM    fm_l2[i%2][CORE_ID*m] → norm_in[i%2]   [m, N]

步骤 II：
  VPU rmsnorm_m(norm_out[i%2], norm_in[i%2], wt_mm, m, N, eps)

步骤 III：
  TE: MM → DDR    norm_in[i%2] → output_res_ddr[offset]  （残差输出）
  TE: MM → DDR    norm_out[i%2] → out_ddr[offset]         （归一化输出）
```

### 6.2 GemmaRMSNorm — 版本 A（广播，基于 `rmsnorm_pipeline`）

在版本 B 基础上，步骤 III 之后增加广播步骤：

```
步骤 IV（核内只有 CORE0 执行）：
  sync_te()                                  // 等待本 cluster 所有 Core 的 TE 写回完成
  TEC0 line_copy<DDR→DDR>:                  // 异步广播本 cluster 本 tile 到对端 Die
      src = out_local_ddr[cluster_global_row * N]
      dst = out_peer_ddr[cluster_global_row * N]
      shape: [cluster_rows, N]
  // Core 1/2/3 不等待广播，继续下一 tile 计算（流水线）


收尾：
  sync_tec()                                 // 等待最后一轮广播完成
```

> **关键**：步骤 IV 必须使用 `sync_te()`（Cluster 级）而非 `fence_te()`（Core 级），才能确保其余三个 Core 的 TE 写回已落盘，广播时源数据有效。

### 6.3 RMSNormGated — 版本 B（独立）

在 RMSNorm 基础上，增加门控加载、激活与逐元素乘法：

```
步骤 0（启动时执行一次）：
  TE: DDR → MM    wt_ddr → wt_mm    [1, N/2]

步骤 I（对每个 tile i）：
  CORE0 TEC0: DDR → L2    fm_ddr[offset] → fm_l2[i%2]      [m×4, N/2]  // 归一化分支
  CORE0 TEC0: DDR → L2    gate_ddr[offset] → gate_l2[i%2]  [m×4, N/2]  // 门控分支
  （两个 TEC0 传输需用不同 Notify ID 区分，或串行发起）

  TE: L2 → MM    fm_l2[i%2][CORE_ID*m] → norm_in[i%2]    [m, N/2]
  TE: L2 → MM    gate_l2[i%2][CORE_ID*m] → gate_mm[i%2]  [m, N/2]

步骤 II：
  VPU rmsnorm_m(norm_out[i%2], norm_in[i%2], wt_mm, m, N/2, eps)

步骤 III：
  VPU silu_m(gate_out[i%2], gate_mm[i%2], m, N/2)  // 门控激活

步骤 IV：
  VPU mul_mm(final_out[i%2], norm_out[i%2], gate_out[i%2], m, N/2)

步骤 V：
  TE: MM → DDR    final_out[i%2] → out_ddr[offset]    [m, N/2]
```

### 6.4 RMSNormGated — 版本 A（广播）

在版本 B 基础上，步骤 V 之后增加与 §6.2 相同的广播步骤，`N` 替换为 `N/2`（广播数据量减半）。

---

## 7. 现有代码问题

以下问题在现有代码中已被识别，新算子开发时需特别注意：

### 7.1 dtype 编码不一致（重要）

`rmsnorm_kernel.ac` 中 `rmsnorm_die_f16()` 的实际映射：

| dtype | `rmsnorm_kernel.ac`（现有，有误） | `rmsnorm_broadcast_kernel.ac`（现有，正确） |
|-------|----------------------------------|---------------------------------------------|
| 0 | float16 ✓ | float16 ✓ |
| 1 | **float32**（注释写 bfloat16）❌ | bfloat16 ✓ |
| 2 | bfloat16 | 不支持 |

**新算子统一采用 `0=float16, 1=bfloat16`**，与 `rmsnorm_broadcast_kernel.ac` 及 `op_interface.md` 一致。旧内核的修复作为独立任务处理，不在本阶段范围内。

### 7.2 广播退化路径的 Notify ID 硬编码

`rmsnorm_standalone`（`rmsnorm_broadcast_kernel.ac`）直接使用字面量：

```c
uint16_t te_tec0[2] = {20, 21};
uint16_t tec0_te[2] = {38, 39};
```

而 `rmsnorm_kernel.ac` 正确地从 `NotifyConfig` 结构体读取。若 `vllm_fusedop_init.ac` 变更 Notify ID 分配，此处将静默出错。**新算子统一使用 `NotifyConfig` 读取 Notify ID。**

### 7.3 `sync_te()` vs `fence_te()` 正确性约束

`rmsnorm_pipeline` 在广播前使用 `sync_te()`（Cluster 级）而非 `fence_te()`（Core 级）。这是正确的：`fence_te()` 只等待当前 Core（Core 0）的 TE，不等 Core 1/2/3 的写回，会导致广播源数据不完整。所有新版本 A 流水线路径必须沿用此模式。

### 7.4 `rmsnorm_run_die_v2` 中的硬编码字节偏移

```c
// float16/bfloat16 路径：
(void*)((char*)out_ddr1 + ((seq_len + 1) / 2) * dim_size * 2)  // 硬编码 *2
// float32 路径（仅因错误 dtype 映射存在）：
(void*)((char*)out_ddr1 + ((seq_len + 1) / 2) * dim_size * 4)  // 硬编码 *4
```

新算子应通过 `type_bytes` 变量统一处理：

```c
int type_bytes = (dtype == 0 || dtype == 1) ? 2 : 4;
void* fm_ddr1_off = (char*)fm_ddr1 + (uint64_t)m0 * dim_size * type_bytes;
```

### 7.5 版本 A 不含残差融合

现有 `rmsnorm_broadcast_kernel.ac` 无 `input_residual` 参数，无残差加法逻辑。若 GemmaRMSNorm 的版本 A 需要残差融合，须在 `rmsnorm_pipeline` 模板中补充残差加载与累加步骤（参照 `rmsnorm_kernel.ac` 的实现）。

---

## 8. 待客户确认的开放问题

以下问题需要与 Lin Feng（需求方）确认后方可开始编码：

| # | 问题 | 背景 | 影响范围 |
|---|------|------|---------|

| **Q1** | RMSNormGated 的门控激活函数是 **SiLU** 还是 **Sigmoid**？是否需要编译时模板参数或运行时 `int gate_act` 参数支持切换？ | 不同模型（如 Mistral MoE vs. 标准 GLU）使用不同激活 | `rmsnorm_gated_kernel.ac` 核心计算步骤，Host 接口 |
| **Q2** | GemmaRMSNorm 的版本 A（broadcast）是否需要**残差融合**（`in_res`/`output_res`）？ | 现有 broadcast 内核无残差路径；添加会增加开发量 | `gemma_rmsnorm_run_die_broadcast` 内核接口 |
| **Q3** | RMSNormGated 的版本 A（broadcast）是否需要残差融合？ | 同上 | `rmsnorm_gated_run_die_broadcast` 内核接口 |
| **Q4** | `rmsnorm_kernel.ac` 中 `dtype=1` 映射到 float32 的**历史 bug** 是否在本阶段一并修复？（会是破坏性变更，影响现有调用方） | 若修复须同步更新所有调用 `launch_add_rmsnorm_die_passthrough` 且传 `dtype=1` 的上层代码 | `rmsnorm_kernel.ac`、`rmsnorm_host.cpp` |
| **Q5** | `rmsnorm_broadcast_kernel.ac` 中 Notify ID 硬编码的修复是否在本阶段完成，还是作为独立任务？ | 当前功能正确但脆弱；建议在本阶段修复以免后患 | `rmsnorm_broadcast_kernel.ac` |

---

*文档版本：v0.1（初稿，待评审）*
*下一步：客户评审开放问题 → 确认后进入编码阶段*

