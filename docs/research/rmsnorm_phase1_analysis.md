# Phase 1 RMSNorm Analysis — GemmaRMSNorm and RMSNormGated on Evas E200

**Date**: 2026-03-15
**Scope**: Design phase only. No code changes.
**Target**: f16/bf16 support. Two versions: A = broadcast+allgather, B = independent.
**Sources**: `rmsnorm_kernel.ac`, `rmsnorm_broadcast_kernel.ac`, `rmsnorm_host.cpp`,
`rmsnorm_broadcast_host.cpp`, `op_interface.md`

---

## Table of Contents

1. [How Existing Kernels Map to Version A and B](#1-how-existing-kernels-map-to-version-a-and-b)
2. [Changes Needed for GemmaRMSNorm](#2-changes-needed-for-gemma-rmsnorm-weight--1)
3. [Changes Needed for RMSNormGated](#3-changes-needed-for-rmsnormgated)
4. [dtype Inconsistency Between Kernels](#4-dtype-inconsistency-between-kernels)
5. [Existing Code Concerns](#5-existing-code-concerns)
6. [op_interface.md Key Details](#6-op_interfacemd-key-details)
7. [Proposed Interface Signatures](#7-proposed-interface-signatures)

---

## 1. How Existing Kernels Map to Version A and B

There are two existing kernel files that map cleanly to the two requested versions:

| Version | Description | AC File | Host File | Kernel Entry |
|---------|-------------|---------|-----------|--------------|
| **B — Independent** | Each die computes its half independently | `rmsnorm_kernel.ac` | `rmsnorm_host.cpp` | `rmsnorm_run_die` / `rmsnorm_run_die_v2` |
| **A — Broadcast+Allgather** | Each die computes its half then broadcasts to peer | `rmsnorm_broadcast_kernel.ac` | `rmsnorm_broadcast_host.cpp` | `rmsnorm_run_die_broadcast` |

### Version B — `rmsnorm_kernel.ac`

Core template function: `rmsnorm(output_ddr, output_res_ddr, input_fm, input_wt, input_residual, M, N, output_gather, eps)`

**Execution flow per tile:**
1. CORE_ID==0: TEC0 loads tile from DDR → L2 (double-buffered, `fm_l2[i%2]`)
2. If residual add: CORE_ID==0: TEC0 loads residual DDR → L2 with `REDUCE_ACC` (in-place adds to same L2 buffer)
3. TE copies L2 → MM per core (`fm_l2 + m*N*CORE_ID → norm_in`)
4. VISA `rmsnorm_m(norm_out, norm_in, wt_mm, m, N, eps)` computes RMSNorm
5. TE writes `norm_in` → `output_res_ddr` (pre-norm sum = updated residual)
6. TE writes `norm_out` → `output_ddr` (normalized result)
7. Optional: TE writes `norm_out` → `output_gather` (allgather output buffer)

**Memory layout (MM offsets):**
```
norm_mm_ping_in  = 0x000000   norm_mm_pong_in  = 0x050000
norm_mm_ping_out = 0x0F0000   norm_mm_pong_out = 0x0A0000
wt_mm            = 0x170000
fm_l2_ping       = 0x000000   fm_l2_pong       = 0x140000  (L2)
residual_l2_ping = 0x280000   residual_l2_pong = 0x3C0000  (L2)
```

**Dual-die entry** `rmsnorm_run_die_v2`: Die0 handles rows `[0, ceil(M/2))`, Die1 handles `[ceil(M/2), M)`. Dies are fully independent; results stay on their own die.

**Notify IDs**: read from `NotifyConfig` struct (`notify_ids.te2tec_out`, `notify_ids.tec2te_fm`).

### Version A — `rmsnorm_broadcast_kernel.ac`

Adds a per-tile TEC0 DDR→DDR broadcast to the peer die after each tile is written back locally.

Two sub-paths:

**Degenerate path** (M < 64, `SMALL_M_THRESHOLD`): Both dies compute the full M independently using `rmsnorm_standalone`. No cross-die transfer. Output on each die is independent (not guaranteed to equal the other die).

**Pipeline path** (M ≥ 64): Uses `rmsnorm_pipeline`.

**Execution flow per tile (pipeline path):**
1. CORE_ID==0: TEC0 loads local half tile DDR → L2
2. TE copies L2 → MM per core
3. VISA `rmsnorm_m` computes
4. `skip_write` flag handles boundary rows (avoids `continue` to prevent sync deadlock)
5. Non-skipped cores: TE writes `norm_out` → local DDR at global row position
6. `sync_te()` (cluster-level) waits for ALL cores' TE writes to complete
7. CORE_ID==0: async TEC0 `line_copy<TEC0, DDR, DDR>` broadcasts this cluster's tile from local DDR → peer die's DDR
8. Core 1/2/3 do not wait for broadcast — pipeline continues to next tile
9. After all tiles: `sync_tec()` waits for final broadcast to complete

**Key synchronization note**: Step 6 uses `sync_te()` (cluster-level) NOT `fence_te()` (core-level). Using `fence_te()` would only wait for CORE_ID==0's TE, missing writes from cores 1/2/3, causing the broadcast to read stale data.

**Notify IDs (hardcoded literals — potential concern, see §5):**
```
te_tec0[2] = {20, 21}
tec0_te[2] = {38, 39}
tec0_bc[2] = {50, 51}  (broadcast-dedicated, currently unused/commented)
```

**Result**: After completion, both dies hold the full M×N output. This is the allgather property.

---

## 2. Changes Needed for GemmaRMSNorm (weight + 1)

### Mathematical difference

Standard RMSNorm: `out = (x / RMS(x)) * γ`

GemmaRMSNorm: `out = (x / RMS(x)) * (γ + 1)`

The only difference is adding 1.0 to each weight element before the per-element multiply.


### Kernel change — Option A: pre-transform weight (recommended)

After the existing weight load:
```c
line_copy<TE, DDR, MM>(input_wt, wt_mm, visa::make_tuple(1, N));
```

Insert one VISA VPU instruction before the tile loop:
```c
add_mf<CORE0>(mm_ptr(wt_mm), mm_ptr(wt_mm), 1.0f, 1, N);
```

This adds 1.0 to all N elements of `wt_mm` in-place, once per kernel launch. All subsequent tiles share the modified weight. No changes to the tile loop body.

**This change is required identically in all three core functions:**
- `rmsnorm()` in `rmsnorm_kernel.ac` (Version B)
- `rmsnorm_standalone()` in `rmsnorm_broadcast_kernel.ac` (Version A degenerate path)
- `rmsnorm_pipeline()` in `rmsnorm_broadcast_kernel.ac` (Version A pipeline path)

### Option B: fused via `rmsnorm_m_withFactor`

VISA provides `rmsnorm_m_withFactor(out, in, factor, M, N, eps)` which multiplies by a scalar factor. This is a per-tensor scalar, not per-element, so it cannot implement per-element `(γ+1)`. **Option B is not applicable here.**

### Signature

No new public parameters needed. GemmaRMSNorm is a distinct kernel type (operator semantic), not a runtime flag. Proposed names:
- `gemma_rmsnorm_run_die` (Version B single-die entry)
- `gemma_rmsnorm_run_die_v2` (Version B dual-die entry)
- `gemma_rmsnorm_run_die_broadcast` (Version A entry)

Host launchers:
- `launch_gemma_rmsnorm_die_passthrough(...)` — same signature as `launch_add_rmsnorm_die_passthrough`
- `launch_gemma_rmsnorm_broadcast_passthrough(...)` — same signature as `launch_rmsnorm_broadcast_passthrough`

---

## 3. Changes Needed for RMSNormGated

### Mathematical definition

```
x_norm, x_gate = split(x, axis=-1)       # each half: [M, N/2]
out = rmsnorm(x_norm, γ, eps) * silu(x_gate)
```

(Variant: some models use `sigmoid` instead of `silu` for the gate. The activation function should be a compile-time template parameter or runtime switch.)

### Structural differences from standard RMSNorm

| Aspect | Standard RMSNorm | RMSNormGated |
|--------|------------------|--------------|
| Input shape | `[M, N]` | `x_norm: [M, N/2]`, `x_gate: [M, N/2]` |
| Weight shape | `[1, N]` | `[1, N/2]` |
| Output shape | `[M, N]` | `[M, N/2]` |
| MM buffers | norm_in, norm_out, wt | norm_in, norm_out, wt, gate_in, gate_out |
| VPU ops per tile | `rmsnorm_m` | `rmsnorm_m` + `silu_m` + `mul_mm` |

### New parameter

```c
void* input_gate    // DDR pointer to gate input [M, N/2]
```

Added after `input_fm` in the function signature. `dim_size` passed to the kernel remains the half-width `N/2` (both norm and gate inputs have the same width).

### Additional MM buffer regions needed

Per core: two double-buffered gate buffers of size `m_tile × (N/2) × dtype_bytes`.

With `m_tile = 16`, `N/2 = 2048` (model hidden 4096), `dtype_bytes = 2`:
```
gate_mm_ping = m_tile × 2048 × 2 = 65536 bytes = 64 KB
gate_mm_pong = 64 KB
Total additional MM: 128 KB per core
```

Current MM usage tops out around `0x170000 + N×2 ≈ 1.39 MB` for N=4096. Adding 128 KB brings total to ~1.51 MB, which **exceeds the 1.5 MB MM limit** for N=4096 at m_tile=16.

**Mitigation options (decision required):**
1. Reduce `m_tile` from 16 to 8 when N is large (reduces norm/gate buffers proportionally)
2. Use in-place gate computation: compute `silu` on `gate_in` in-place, then `mul_mm` against `norm_out`; reuse one of the norm ping/pong buffers for gate (overlapping lifetimes allow this)
3. Stream gate load and compute in a tighter pipeline to avoid holding two full tile copies simultaneously

### Tile loop changes

After step 3 (VISA `rmsnorm_m` computes `norm_out`), add:

```
// Gate path (interleaved with norm pipeline):
A. TEC0: load gate tile DDR → L2 (can share TEC0 slot with fm load if notify IDs permit,
         or use a separate notify pair)
B. TE: gate_l2 + m*N/2*CORE_ID → gate_mm
C. silu_m(gate_out, gate_mm, m, N/2)
D. mul_mm(final_out, norm_out, gate_out, m, N/2)
E. TE: final_out → output_ddr
```

Steps A/B can overlap with the existing norm computation if gate load is issued before `rmsnorm_m` and a separate notify chain is used. This requires one additional notify ID pair.


### Broadcast variant (Version A)

Same structure as `rmsnorm_pipeline` but operate on `N/2`-wide data. The broadcast tile size in the `line_copy<TEC0, DDR, DDR>` step changes from `cluster_rows × N` to `cluster_rows × (N/2)`, cutting cross-die traffic in half.

---

## 4. dtype Inconsistency Between Kernels

### The inconsistency

| dtype code | `rmsnorm_kernel.ac` (`rmsnorm_die_f16`) | `rmsnorm_broadcast_kernel.ac` | op_interface.md |
|---|---|---|---|
| 0 | `float16` ✓ | `float16` ✓ | `float16` ✓ |
| 1 | **`float32`** ← bug | `bfloat16` ✓ | `bfloat16` ✓ |
| 2 | `bfloat16` | unsupported | — |

### Root cause

`rmsnorm_kernel.ac` was written before the dtype convention was standardized. Inside `rmsnorm_die_f16`:
```c
if (dtype == 0)        // float16        ✓
else if (dtype == 1)   // float32        ← caller expects bfloat16
else if (dtype == 2)   // bfloat16       ← unreachable from rmsnorm_run_die entry
```

`rmsnorm_run_die` (the entry kernel) has this comment: `// float16 or bfloat16` for the `dtype == 0 || dtype == 1` branch, which is misleading — dtype=1 actually invokes float32.

`rmsnorm_run_die_v2` has a correct separate branch for dtype=1 that uses `* 4` byte offset (float32 size), so it is internally consistent with the wrong mapping but will misbehave if a caller passes `1` expecting bfloat16.

### Impact and recommendation for Phase 1

Phase 1 targets f16/bf16 only. **Standardize on:**
- `dtype=0` → `float16`
- `dtype=1` → `bfloat16`

This matches `rmsnorm_broadcast_kernel.ac` and all other operators in `op_interface.md`.

New kernels (GemmaRMSNorm, RMSNormGated) must use the correct mapping. The existing `rmsnorm_kernel.ac` bug should be noted but fixing it is a separate task (it is a breaking change for any current callers passing dtype=1 expecting float32).

---

## 5. Existing Code Concerns

### 5.1 Hardcoded notify IDs in broadcast kernel

`rmsnorm_standalone` in `rmsnorm_broadcast_kernel.ac` uses raw integer literals:
```c
uint16_t te_tec0[2] = {20, 21};
uint16_t tec0_te[2] = {38, 39};
```

`rmsnorm_kernel.ac` correctly reads from `NotifyConfig` struct. If notify IDs are ever reassigned in `vllm_fusedop_init.ac`, the broadcast standalone path will silently use wrong IDs and produce memory corruption or deadlock.

**Recommendation**: Replace literals in `rmsnorm_standalone` with `NotifyConfig` fields, matching the pattern in `rmsnorm_kernel.ac`.

### 5.2 `sync_te()` vs `fence_te()` correctness — fragile but correct

`rmsnorm_pipeline` uses `sync_te()` (cluster-level barrier) before the broadcast step. This is correct because it waits for all four cores' TE writes. Using `fence_te()` (core-level) would only wait for CORE_ID==0's TE and would broadcast stale data from other cores.

The comment in the code explains this explicitly. However, this is a correctness landmine for future maintainers. Any GemmaRMSNorm/RMSNormGated broadcast variant must replicate this pattern exactly.

### 5.3 `rmsnorm_run_die_v2` — hardcoded byte stride for dtype=1

```c
// Die1 output offset for float16/bfloat16 (dtype 0 or 2):
(void*)((char*)out_ddr1 + ((seq_len + 1) / 2) * dim_size * 2)

// Die1 output offset for float32 (dtype 1 in the broken mapping):
(void*)((char*)out_ddr1 + ((seq_len + 1) / 2) * dim_size * 4)
```

This is an artifact of the dtype inconsistency. After fixing the dtype mapping, a single branch with `type_bytes` computed from dtype suffices.

### 5.4 Residual output semantics — naming confusion

In `rmsnorm_kernel.ac:92–94`:
```c
line_copy<TE, MM, DDR>(norm_in[i % 2], output_res_ddr + offset, ...);  // writes pre-norm input
line_copy<TE, MM, DDR>(norm_out[i % 2], output_ddr + offset, ...);     // writes normalized output
```

`norm_in` after the residual-add step holds `fm + in_residual`. This is written to `output_res_ddr`, which per op_interface.md is correct ("residual = in + [residual]"). The name `norm_in` suggests normalized input, but it actually contains the pre-norm sum. Not a bug, but confusing.

### 5.5 broadcast kernel has no residual path

`rmsnorm_broadcast_kernel.ac` (`rmsnorm_standalone`, `rmsnorm_pipeline`) has no `input_residual` parameter and no residual add logic. The standard `rmsnorm_kernel.ac` supports residual fusion. If GemmaRMSNorm or RMSNormGated need residual fusion in the broadcast version, it must be added.

### 5.6 `continue` vs `skip_write` pattern — only in broadcast

`rmsnorm_pipeline` replaces `continue` with a `skip_write` boolean flag to ensure all cores reach `sync_te()`. The standard `rmsnorm()` in `rmsnorm_kernel.ac` still uses `continue` on boundary tiles, which is safe there because no cluster-level sync follows. New kernels based on the pipeline pattern must use `skip_write`, not `continue`.

---

## 6. op_interface.md Key Details

### RMSNorm single-die host interface (existing, to be extended)

```c
void launch_add_rmsnorm_die_passthrough(
    void* out_ddr0,         // normalized output [M, N]

    void* output_res_ddr0,  // updated residual = fm + in_residual [M, N]
    void* fm_ddr0,          // input feature matrix [M, N]
    void* wt_ddr0,          // weight [1, N]
    void* in_res_ddr0,      // residual input [M, N]; NULL = no residual add
    float eps,
    int seq_len0,           // M
    int dim_size0,          // N
    int dtype,              // 0=float16, 1=bfloat16
    int device_id,
    int m_tile_size);       // <=0 means kernel decides
```

### RMSNorm dual-die kernel (existing, to be extended)

```c
__global__ void rmsnorm_run_die_v2(
    void* out_ddr0, void* output_res_ddr0, void* fm_ddr0, void* wt_ddr0, void* in_res_ddr0,
    void* out_ddr1, void* output_res_ddr1, void* fm_ddr1, void* wt_ddr1, void* in_res_ddr1,
    int seq_len, int dim_size, int dtype = 1, float eps = 1e-5);
```

Data split: `seq_len` (M dimension) split evenly between dies. Die0: rows `[0, ceil(M/2))`, Die1: rows `[ceil(M/2), M)`.

### RMSNorm broadcast kernel (existing)

```c
__GLOBAL__ void rmsnorm_run_die_broadcast(
    void* out_ddr0, void* out_ddr1,   // both dies write full M×N result
    void* fm_ddr0,  void* fm_ddr1,
    void* wt_ddr0,  void* wt_ddr1,
    int seq_len, int dim_size, int dtype, float eps);
```

Host launcher: `launch_rmsnorm_broadcast_passthrough(out_ddr0, out_ddr1, fm_ddr0, fm_ddr1, wt_ddr0, wt_ddr1, seq_len, dim_size, dtype, eps, device_id)`.

Must launch with `evConfigureCall(8, 4)` (8 clusters × 4 cores = both dies).

### dtype convention (all operators in op_interface.md)

```
dtype = 0  →  float16
dtype = 1  →  bfloat16
```

---

## 7. Proposed Interface Signatures

Following the existing naming conventions and parameter ordering:

### GemmaRMSNorm — Version B (independent)

```c
// Single-die host launcher
void launch_gemma_rmsnorm_die_passthrough(
    void* out_ddr0, void* output_res_ddr0, void* fm_ddr0, void* wt_ddr0,
    void* in_res_ddr0, float eps, int seq_len0, int dim_size0,
    int dtype, int device_id, int m_tile_size = 16);

// Dual-die kernel (new)
__GLOBAL__ void gemma_rmsnorm_run_die_v2(
    void* out_ddr0, void* output_res_ddr0, void* fm_ddr0, void* wt_ddr0, void* in_res_ddr0,
    void* out_ddr1, void* output_res_ddr1, void* fm_ddr1, void* wt_ddr1, void* in_res_ddr1,
    int seq_len, int dim_size, int dtype = 1, float eps = 1e-5);
```

### GemmaRMSNorm — Version A (broadcast+allgather)

```c
// Dual-die kernel (new)
__GLOBAL__ void gemma_rmsnorm_run_die_broadcast(
    void* out_ddr0, void* out_ddr1,
    void* fm_ddr0,  void* fm_ddr1,
    void* wt_ddr0,  void* wt_ddr1,
    int seq_len, int dim_size, int dtype, float eps);

// Host launcher (new)
void launch_gemma_rmsnorm_broadcast_passthrough(
    void* out_ddr0, void* out_ddr1,
    void* fm_ddr0,  void* fm_ddr1,
    void* wt_ddr0,  void* wt_ddr1,
    int seq_len, int dim_size, int dtype, float eps, int device_id);
```

### RMSNormGated — Version B (independent)

```c
// Single-die host launcher
void launch_rmsnorm_gated_die_passthrough(
    void* out_ddr0,         // output [M, N/2]
    void* fm_ddr0,          // norm input [M, N/2]
    void* gate_ddr0,        // gate input [M, N/2]
    void* wt_ddr0,          // weight [1, N/2]
    float eps, int seq_len0, int dim_size0,  // dim_size0 = N/2
    int dtype, int device_id, int m_tile_size = 16);

// Dual-die kernel (new)
__GLOBAL__ void rmsnorm_gated_run_die_v2(
    void* out_ddr0, void* fm_ddr0, void* gate_ddr0, void* wt_ddr0,
    void* out_ddr1, void* fm_ddr1, void* gate_ddr1, void* wt_ddr1,
    int seq_len, int dim_size, int dtype = 1, float eps = 1e-5);
```

### RMSNormGated — Version A (broadcast+allgather)

```c
// Dual-die kernel (new)
__GLOBAL__ void rmsnorm_gated_run_die_broadcast(
    void* out_ddr0,  void* out_ddr1,
    void* fm_ddr0,   void* fm_ddr1,
    void* gate_ddr0, void* gate_ddr1,
    void* wt_ddr0,   void* wt_ddr1,
    int seq_len, int dim_size, int dtype, float eps);

// Host launcher (new)
void launch_rmsnorm_gated_broadcast_passthrough(
    void* out_ddr0,  void* out_ddr1,
    void* fm_ddr0,   void* fm_ddr1,
    void* gate_ddr0, void* gate_ddr1,
    void* wt_ddr0,   void* wt_ddr1,
    int seq_len, int dim_size, int dtype, float eps, int device_id);
```

---

## Open Questions for Team

1. **Gate activation function**: `silu` or `sigmoid` for RMSNormGated? Should it be a compile-time template parameter or a runtime `int gate_act` flag?

2. **MM memory for RMSNormGated**: With N=4096 (dim_size=2048 per half) and m_tile=16, adding gate buffers pushes MM usage to ~1.51 MB, slightly over the 1.5 MB limit. Decision needed: reduce m_tile, reuse a ping/pong buffer, or accept N ≤ 3072 constraint?

3. **Residual fusion in broadcast variant**: Should GemmaRMSNorm or RMSNormGated broadcast versions support `in_res`/`output_res` residual parameters (not currently in `rmsnorm_broadcast_kernel.ac`)?

4. **Notify ID literals in broadcast standalone path**: Fix the hardcoded `{20, 21}` / `{38, 39}` as part of Phase 1, or defer to a separate cleanup task?

5. **Existing dtype bug in `rmsnorm_kernel.ac`**: Fix the dtype=1 mapping (float32 → bfloat16) as part of Phase 1, or leave for a separate breaking-change task?

---

*Analysis written: 2026-03-15*
*Based on: rmsnorm_kernel.ac, rmsnorm_broadcast_kernel.ac, rmsnorm_host.cpp, rmsnorm_broadcast_host.cpp, op_interface.md*

