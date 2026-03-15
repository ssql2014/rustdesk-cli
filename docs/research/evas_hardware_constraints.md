# Evas E200 Hardware Constraints Reference

Extracted from `evas_learning_report.md` Sections 7 and 9 for use in test validation.

---

## 1. Hardware Constants

```
CLUSTER_NUM      = 8       // Clusters per chip
CORE_NUM         = 4       // Cores per cluster
DIE_NUM          = 2       // Dies per SoC
CLUSTER_PER_DIE  = 4       // Clusters per die
L2_SIZE          = 9 MB    // L2 per cluster
MM_SIZE          = 1.5 MB  // MM (L1) per core
AM_SIZE          = 0.25 MB // AM (L1) per core
```

## 2. Runtime Limits

| Constraint | Value |
|------------|-------|
| Max gather/scatter indices | 2048 |
| Notify ID range | 0–41 (42 total) |
| Device-side stack | 1 MB (fixed, in global memory) |
| Max kernel argument types | No long/double in `__global__` |
| head_dim (attention) | {64, 128} only |
| kv_head_cnt_soc | ≥ 2 and must be even |
| block_size | Must satisfy `block_size * head_dim * bytes ≤ gather index limits` |

## 3. ME Alignment Requirements

| Data | Location | Alignment |
|------|----------|-----------|
| Feature maps (FM) | MM | Element bit-width |
| Weight matrices | MM | 64 bytes |
| Bias | MM | 64 bytes |
| Output | AM | 64 bytes |

## 4. ME Input Shape Requirements

- **M ≥ 32** (amortizes weight-loading overhead)
- **N**: must be multiple of 32 (or 128 for S8S4 mode)
- **K**: no strict alignment requirement

## 5. ME Throughput Modes

| Mode | FM dtype | Weight dtype | Accum dtype | Tile |
|------|----------|-------------|-------------|------|
| VMC_F16TOF32_M1K64N64 | float16 | float16 | float32 | 1×64×64 |
| VMC_S8S4TOS32_M1K128N128 | int8 | int4 | int32 | 1×128×128 |
| (others) | BF16, FP8 | BF16, FP8 | FP32 | varies |

## 6. VE Minimum Tile Sizes

| dtype | Minimum elements for full utilization |
|-------|--------------------------------------|
| float16 / bfloat16 | 512 |
| int8 | 1024 |
| float32 | 256 |

- Use full LMUL=8 RISC-V vector loads/stores for 1024-bit throughput
- TE load/store from MM has ~7-cycle latency; avoid back-to-back LD→compute on same address

## 7. Bank Conflict Rules

**Conflict condition**: two or more simultaneous accesses to the **same bank group and same slice**.

### ME Weight-Loading Conflicts

| Weight dtype | Rows/cycle | Risky stride (elements) | Safe stride |
|-------------|------------|--------------------------|-------------|
| float16 | 2 | 128 (= 4 × 512-bit bank groups) | 64 |
| int8 | 4 | 256 (all 4 rows hit same bank group) | 64 |

### Mitigation Strategies

- Distribute FM, Weight, Bias, Output to **different MM slices**
- Store intermediate VE results in a slice separate from ME inputs
- Use `static_assert` or address arithmetic to verify slice placement at compile time

## 8. Memory Residency Limits (per core)

```
FM in MM:      ≤ 512 KB
Weight in MM:  ≤ 512 KB
Bias in MM:    ≤ 8 KB
Output in AM:  ≤ 128 KB  (PAM ≤ 256 KB)
Total MM:      ≤ 1.5 MB
```

## 9. Tiling Factors (GEMM example: 512×4096×4096)

- **M-tile**: 128 per core (4-core parallelism inside cluster)
- **K-tile**: 512 chunks (determines inner-loop L2 residency)
- **N-tile**: 128 or 256 chunks (determines outer-loop cluster partition)

## 10. TEC Bandwidth

- TEC0 (2 channels) has **2× the DDR bandwidth** of TEC1 (1 channel)
- Prefer TEC0 for heavy DDR traffic
- Use larger burst sizes for higher DDR channel utilization
- Partition weights across clusters (each cluster owns its slice) rather than broadcasting

## 11. Double-Buffering Pattern

```
Buffer A, Buffer B (each holds one tile):

Iteration 0:  Load tile-0 → Buffer A           [TE/TEC busy]
Iteration 1:  Compute tile-0 (A)  |  Load tile-1 → Buffer B  [ME+TEC overlap]
Iteration 2:  Compute tile-1 (B)  |  Load tile-2 → Buffer A
              ...
```

- ~50% tail overhead reduction vs non-pipelined
- ME/VE auto-detect data dependencies on L1 addresses — no manual FENCE needed for sequential ops on same address range
- Manual sync still needed across engines (ME → VE output going to another core via TE)

## 12. Sync Mechanism Selection

| Mechanism | Use Case | Overhead |
|-----------|----------|----------|
| `sync()` | Cluster barrier when all cores must agree | High (stall all cores) |
| `FENCE_ME/VE/TE` | Engine ordering within one core | Low (waits for one engine) |
| Notify | Producer-consumer pipeline overlap | Very low (MCU polls register) |

**Prefer Notify** for producer-consumer overlap; use `sync()` only for full cluster rendezvous.

## 13. Gather/Scatter Constraints

- Must call `copy_indexram()` to load indices before issuing operation
- Index values are `uint32_t` byte offsets relative to buffer base
- Base address offsets: DDR → `0x1000000000UL`, L2 → `0x42000000`, MM → `0x200000`
- Index buffer: 0x2000 bytes total (2 × 0x1000 banks), max **2048 indices**
- Cannot issue to same engine until current index_ram transfer is complete
- Different engines can run gather/scatter in parallel
- TEC preferred over TES for performance

## 14. Attention Kernel Constraints (current codebase)

- `q_head_cnt` must equal **8**
- `kv_head_cnt_die` must equal **1** (MQA only per die)
- `kv_head_cnt_soc` must equal **2**
- KV gather owners hardcoded to `CLUSTER_ID==0` and `CLUSTER_ID==3`
- Known bug: `set_kv_block_idx` stride ignores `kv_head_cnt` (always uses `block_size * head_dim * bytes`)
