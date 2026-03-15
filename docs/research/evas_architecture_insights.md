# Evas Architecture Insights for New Operator Design

This note distills the key architectural lessons from [evas_learning_report.md](/Users/qlss/Documents/Projects/rustdesk-cli/docs/research/evas_learning_report.md) into an operator-design playbook. The goal is not to restate the manuals, but to capture the patterns that should guide future kernel work in `.ac`.

## Architecture Corrections from Meeting (2026-03-15)

Jerry confirmed several architectural points directly with the Evas team. These refine the mental model used below and should be treated as authoritative.

### Confirmed facts

- each chip has 2 dies
- each die has 4 clusters
- each cluster has multiple cores
- each core is composed of:
  - VPU, implemented as an Andes RISC-V core
  - Tensor Core
  - DMA
  - Scheduler
- the Scheduler reads VISA instructions in order but executes them out of order
- VISA instructions are macro instructions, built from lower-level C and RISC-V instructions
- the overall machine follows a dataflow execution model

### Design implications

- Out-of-order execution inside the core scheduler means operator code should expose independence between transfers, tensor-core work, and vector work whenever possible. Independent VISA instructions can overlap automatically if we do not serialize them unnecessarily with avoidable fences or coarse barriers.
- The dataflow model reinforces the staged transfer pattern already seen in the kernels: DDR -> L2 -> MM/AM is not just a memory hierarchy detail, it is the mechanism that keeps compute fed. Operator design should focus on keeping the next tile available before the current tile finishes.
- Because VISA is a macro-instruction layer rather than a one-to-one mapping to raw hardware operations, there is an abstraction boundary between AC/C++ source and actual execution. Performance tuning therefore depends on understanding not just which VISA ops are issued, but also how those ops expand into lower-level behavior and where that expansion may introduce hidden dependencies or scheduling limits.
- The scheduler model suggests a practical rule: write kernels so instruction streams contain long runs of movement and compute operations that are logically independent, then use the lightest possible synchronization primitive only at true dependency boundaries.
- The corrected core model also sharpens the division of responsibilities:
  - Tensor Core for dense math
  - VPU for vector and control-heavy work
  - DMA for movement
  - Scheduler for extracting overlap from the issued VISA stream
  This makes operator performance primarily a scheduling and residency problem, not just an arithmetic one.

## 1. Mental Model of the Chip

The E200 is not a flat SIMD device. It is a four-level hierarchy:

- chip: 2 dies
- die: 4 clusters
- cluster: 4 cores + 9 MB shared L2 + TEC engines
- core: private 1.5 MB MM + 0.25 MB AM + ME/VE/TE + MCU

The practical consequence is that every operator needs an explicit decomposition strategy at three levels:

- cluster partitioning: which output tiles or tensor regions each cluster owns
- core partitioning: how work is split among the 4 cores in a cluster
- engine partitioning: which work goes to ME, VE, TE, TEC, and when

The fastest designs treat the chip as a dataflow machine:

- DDR is a streaming backing store
- cluster L2 is the staging and sharing layer
- MM/AM are the true compute-resident working set
- ME and VE consume data only after the MCU has orchestrated residency correctly

If an operator design does not start from this hierarchy, it will usually lose performance to movement overhead before arithmetic becomes the bottleneck.

One correction from the meeting is worth making explicit: the core should be thought of as a scheduled compound processing element, not just a bag of engines. The Scheduler issues VISA macro instructions in order but can execute ready operations out of order, which means the kernel author's job is to present a dependency graph with enough independent work to keep Tensor Core, VPU, and DMA busy concurrently.

## 2. Reusable Execution Pattern

The existing operators suggest a consistent kernel template:

1. Partition output space across clusters and cores.
2. Prefetch the next tile from DDR to L2 with TEC, preferably TEC0 for heavy traffic.
3. Move the current tile from L2 to MM/AM with TE.
4. Run the dense part on ME and the elementwise or normalization part on VE.
5. Write results back through TE and TEC.
6. Overlap step 2 with steps 3-5 using double buffering.

This same pattern appears in linear, MLP, attention, fused GEMM+RoPE, and KV-cache reshape. New operators should default to this staged template unless there is a clear reason not to.

### Design rule

Prefer building operators as a pipeline of staged tiles instead of as a single monolithic kernel body. The hardware is designed to reward pipelining and overlap more than raw instruction density.

The meeting clarification on dataflow execution strengthens this rule. The purpose of staging is not merely to organize memory traffic; it is to keep the dataflow graph supplied so the scheduler always has ready work to issue out of order.

## 3. Memory Placement Strategy

Memory placement is the first design decision, not a later optimization pass.

### What each memory is best for

- MM: primary ME input tiles and VE-friendly local tiles
- AM: accumulators and ME outputs
- L2: cluster-shared staging buffers, prefetch buffers, cross-core exchange
- DDR: persistent tensors, weights, KV cache, final outputs

### Placement heuristics

- Keep feature-map, weight, bias, and temporary outputs in different MM slices when possible to avoid bank conflicts.
- Treat L2 as a software-managed cache, not a passive spill area.
- Size tiles so the steady-state working set stays within MM/AM limits:
  - FM in MM: up to about 512 KB
  - weight tile in MM: up to about 512 KB
  - bias in MM: small, ideally under 8 KB
  - output in AM: ideally under 128 KB

### Practical implication

When starting a new operator, sketch the MM/L2/DDR residency plan before writing any VISA calls. Most implementation bugs and performance failures will trace back to a bad residency plan, not the arithmetic itself.

## 4. Engine Specialization Matters

The chip exposes separate execution resources for compute and movement:

- ME: dense matmul-heavy work
- VE: elementwise transforms, reductions, normalization, activation logic
- TE: local movement among MM, AM, and L2
- TEC: DDR <-> L2 movement, with TEC0 preferred for sustained bandwidth
- TES: cross-die or off-cluster transport when needed

This means operator boundaries should often align with engine boundaries:

- use ME for high-arithmetic-intensity projections
- use VE for post-processing on resident tiles
- avoid moving partially processed data back to DDR just to switch engines

The strongest pattern in the existing codebase is fusion across engine-friendly stages, especially when it removes a DDR round trip. The fused GEMM+Norm+RoPE kernels are the clearest example.

With the corrected core model, it is also useful to reinterpret these blocks in meeting terminology:

- Tensor Core corresponds to the dense math path
- VPU corresponds to vector math and control-oriented work
- DMA corresponds to the movement path
- Scheduler is the mechanism that overlaps them when dependencies allow

That framing is useful when deciding whether an operator should be fused, split, or reordered.

## 5. VISA and `.ac` Programming Model Implications

AC looks like C++17, but effective `.ac` code is much closer to explicit kernel assembly with templates:

- scoped pointers (`mm_ptr`, `am_ptr`, `l2_ptr`, `ddr_ptr`) encode address-space intent
- VISA intrinsics assume the caller already solved layout, alignment, and buffer sizing
- the MCU is responsible for orchestration, triggering, and synchronization

The meeting added an important caveat: VISA is not the raw hardware interface. It is a macro-instruction layer that ultimately lowers into lower-level C and RISC-V instruction sequences. That means source-level intent and runtime behavior are related, but not identical.

### Reusable coding pattern

Good `.ac` kernels seem to separate into three layers:

- launch and topology logic: cluster/core ownership, die split, offsets
- movement schedule: TE/TEC descriptor setup, ping-pong buffers, gather/scatter indices
- math microkernel: ME or VE calls on already-resident tiles

That separation is worth preserving in future operators because it keeps the kernel debuggable. It also makes it easier to reuse the movement schedule with different math cores.

It also gives a clean place to reason about the abstraction boundary:

- topology logic determines the dataflow graph
- movement schedule determines data readiness
- math microkernel determines Tensor Core and VPU utilization

If performance is poor, the problem may be in any of those layers or in how the VISA macro instructions generated from them interact with the scheduler.

## 6. Synchronization Strategy

The report makes a strong case that synchronization should be as local as possible.

### Prefer in this order

- implicit dependency tracking when the same L1 addresses are used sequentially
- `FENCE_ME`, `FENCE_VE`, `FENCE_TE`, `FENCE_TEC0` for single-engine ordering
- notify-based producer/consumer handoff for overlap
- `sync()` only when the whole cluster truly needs a rendezvous
- die or chip-wide barriers only as a last resort

### Why this matters

Large barriers destroy the overlap that the hardware is built for. Notify is especially important for kernels with staged transfer and compute, because it lets one engine progress without stalling the full cluster.

This matters even more under the corrected scheduler model. Since the scheduler can extract overlap from independent VISA instructions automatically, over-synchronizing does double damage: it blocks both explicit pipeline overlap and the scheduler's own out-of-order opportunities.

## 7. Patterns Reused by Existing Operators

### Linear / GEMM

- cluster- and core-partitioned output tiles
- weight prefetch through L2
- double-buffered K-loop
- optional VE epilogue for bias or lightweight post-processing

This is the baseline template for any projection-heavy operator.

### MLP / SwiGLU

- parallel gate and up projections
- VE activation on the resident gate tile
- elementwise fusion before the down projection

The reusable lesson is to keep the two branch projections close in schedule and memory layout so the Hadamard product can happen in-core without a DDR spill.

### RMSNorm

- VE-friendly reduction and scale pattern
- small arithmetic footprint but strong sensitivity to vector utilization and alignment

This is a good example of an op that should often be fused with adjacent stages when the surrounding data is already resident.

### RoPE

- fused into projection kernels instead of emitted as a standalone pass

This is the clearest architecture-level rule from the report: when an operator is structurally cheap but movement-heavy, fuse it into the nearest dense producer or consumer.

### Attention / GQA

- mixed gather/scatter, reduction, softmax, and tile scheduling
- split-KV processing and merge
- hardcoded topology assumptions in the current implementation

This is the most important warning sign in the codebase. Attention stresses every part of the machine at once, so it exposes hidden assumptions early.

## 8. Constraints to Design Around Up Front

These constraints should be part of operator design review before implementation starts:

- MM/AM/L2 capacities are small enough that tiling is mandatory for most LLM workloads.
- ME data placement has alignment and slice-placement requirements; violating them can erase compute throughput through bank conflicts.
- VE needs sufficiently large tiles for full utilization:
  - FP16/BF16: about 512 elements
  - INT8: about 1024 elements
  - FP32: about 256 elements
- Gather/scatter supports only 2048 indices per index buffer and needs explicit index RAM management.
- Notify IDs are limited to 42 total.
- Device stack is fixed at 1 MB, so large local recursion or bulky temporary structures are off-limits.
- Current attention code assumes `q_head_cnt == 8`, `kv_head_cnt_die == 1`, and `kv_head_cnt_soc == 2`; anything more general will require real kernel surgery, not parameter tweaking.

## 9. Optimization Opportunities

### 1. Fuse movement-dominated ops

RoPE already demonstrates the right pattern. The same logic likely applies to:

- residual add fused into nearby norm or projection epilogues
- bias + activation epilogues after GEMM
- small reshapes or transposes when they can be absorbed into producer/consumer layout

### 2. Convert standalone VE passes into GEMM epilogues/prologues

If the data is already in MM/AM after ME, do the VE work there. Avoid a TE/TEC round trip unless the intermediate is reused by multiple later stages.

### 3. Use cluster-local ownership instead of broadcast when possible

The report explicitly recommends partitioning weights across clusters rather than broadcasting them. This should be the default for large static tensors like linear weights.

### 4. Prefer notify-driven overlap over cluster barriers

This is likely one of the highest-leverage improvements for immature kernels. It preserves concurrency without forcing all four cores to idle at the same rendezvous point.

### 5. Treat TEC0 bandwidth as a scarce primary resource

Heavy DDR traffic should be scheduled around TEC0 first. If a kernel depends on multiple large DDR streams, its layout should be redesigned to reduce concurrent demand rather than assuming the hardware will hide it.

## 10. Recommended Design Checklist for New Operators

Before implementing a new operator, answer these questions:

1. What is the cluster-level ownership of outputs and weights?
2. What tiles live in DDR, L2, MM, and AM at steady state?
3. Which stages are ME-bound and which are VE-bound?
4. Where can double buffering overlap TEC with compute?
5. Can any movement-heavy step be fused away?
6. Are MM slices and strides chosen to avoid bank conflicts?
7. Are VE tile sizes large enough for efficient vectorization?
8. Do gather/scatter index counts and notify IDs fit hardware limits?
9. Does the operator require a full-cluster barrier, or can notify/FENCE suffice?
10. Are there hidden topology assumptions, especially in attention-like paths?

## Bottom Line

The E200 rewards explicit orchestration more than generic parallel code. The winning operator pattern is:

- tile aggressively
- stage through L2
- compute from MM/AM
- overlap transfer and compute
- fuse cheap movement-heavy steps
- synchronize locally
- design for bank layout and engine specialization from the start

If we follow that pattern, new operators will fit the chip. If we treat the chip like a generic accelerator with automatic caching and uniform execution costs, they will not.
