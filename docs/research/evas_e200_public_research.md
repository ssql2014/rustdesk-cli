# Research Findings: EVAS Intelligence E200 (Epoch Series)

## 1. Company Overview: EVAS Intelligence (奕行智能)
- **Founded:** January 2022, headquartered in Guangzhou, China.
- **Focus:** High-performance general-purpose AI computing chips (GP-NPU) for autonomous driving (L2+ to L4), robotics, and edge computing.
- **Key Achievement:** Developed the **Epoch** series, the first large-scale mass-produced RISC-V AI compute chips in China.

## 2. Chip Architecture: EVAMIND™
The EVAMIND architecture is a Domain-Specific Architecture (DSA) combining systolic array efficiency with general-purpose flexibility.

### Cluster/Core Topology:
- **Modular Scaling:** Scalable across multiple cores connected via a high-bandwidth **Network-on-Chip (NoC)**, typically using a 2D Mesh or Ring-Mesh topology.
- **Core Heterogeneity:** Each core is a "multi-engine" processor containing five functional units:
    1. **Scalar Engine:** RISC-V management core for control and non-parallel tasks.
    2. **Tensor Engine:** High-density matrix math engine (similar to TPU) optimized for GEMM operations.
    3. **4D Acceleration Engine:** Specialized for data shuffling, tensor transformations (transposition, rotation), and element-wise operations.
    4. **Vector Engine (RVV):** Standard **RISC-V Vector 1.0** unit for fine-grained SIMD tasks.
    5. **VISA Scheduler:** Hardware implementation of the Virtual ISA layer.

## 3. VISA Instruction Set & Programming Model
- **VISA (Virtual Instruction Set Architecture):** 
    - Proprietary intermediate layer between AI compiler and hardware.
    - Enables **macro-instruction scheduling** and **out-of-order (OoO) dispatch**.
    - Future-proofs the chip by allowing new AI operators to be mapped via software updates.
- **Software Stack (ETK):** The EVAS Tool Kit supports standard frameworks (PyTorch, TensorFlow, ONNX).

## 4. Performance & Comparison
- **Performance:** The "E200" configuration delivers **200 TOPS** (INT8) of AI compute. Supports **INT4, INT8, FP8, FP16, and BF16**.
- **Comparison:**
    - **Vs. NVIDIA GPU:** Higher energy efficiency and utilization for BEV and Transformer models.
    - **Vs. Google TPU:** Shares systolic array concept but adds a full RISC-V/RVV vector processor for custom operators.

---
*Note: This research supplements internal documentation found on the NUC machine with public context.*
