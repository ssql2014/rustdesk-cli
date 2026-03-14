# Research: Quantization Strategies for Llama 3 8B

This document details the quantization methods and memory-saving strategies for deploying Llama 3 8B on hardware-constrained remote nodes.

## 1. Quantization Format Overview

| Format | Bit-rate | Primary Ecosystem | Best For |
| :--- | :--- | :--- | :--- |
| **GGUF** | 2-8 bit | `llama.cpp` | CPU/GPU split, cross-platform stability. |
| **AWQ** | 4-bit | `vLLM`, `AutoAWQ` | Fast GPU inference, high logic preservation. |
| **GPTQ** | 4-bit | `AutoGPTQ` | Fast GPU inference (older hardware). |
| **NF4** | 4-bit | `bitsandbytes` | Fine-tuning (QLoRA) and experimentation. |

## 2. Memory vs. Precision (Llama 3 8B)

Calculations are based on 8.03 billion parameters.

- **FP16 (Original):** 16.1 GB. Requires high-end consumer (RTX 3090/4090) or datacenter GPUs.
- **INT8 (GGUF Q8_0):** 8.5 GB. Fits in 10-12GB VRAM. Near-zero quality loss.
- **INT4 (GGUF Q4_K_M):** 5.0 GB. Fits in 8GB VRAM (standard laptop/cloud GPUs). Minor quality loss (1-2%).
- **INT2 (GGUF Q2_K):** 3.1 GB. Fits in 4GB VRAM. **Not recommended** for Llama 3 (logic becomes incoherent).

## 3. Recommended Strategy: GGUF

For the `rustdesk-cli` remote inference use case, **GGUF (Q5_K_M or Q6_K)** is the recommended format.

### Rationale:
1. **Tooling:** The `gguf-py` library provides a pure-Python reader that integrates seamlessly with NumPy.
2. **Hybrid Execution:** GGUF allows "offloading" layers to the GPU while keeping the rest in RAM, providing a fallback if the node's GPU is too small.
3. **Robustness:** Llama 3 is information-dense; GGUF's "K-Quants" (like Q5_K_M) protect critical layers (attention projections) more aggressively than standard 4-bit methods.

## 4. Remote Operator Design: Dequantization

Quantized weights cannot be used directly in a standard `MatMul`. 

- **Approach:** Use **Dequantize-on-the-fly**.
- **Logic:** Load the quantized tensor into RAM. Immediately before the `matmul` operator, convert the required blocks to `float32`.
- **Performance:** While adding a CPU overhead, this significantly reduces the bandwidth required to `push` the model weights via RustDesk (~5GB vs ~16GB).

## 5. Python Implementation (GGUF Reading)

To read weights from a `.gguf` file using NumPy on the remote peer:

```python
import gguf
import numpy as np

def load_weight(gguf_path, tensor_name):
    """Reads and dequantizes a specific tensor."""
    reader = gguf.GGUFReader(gguf_path)
    
    # 1. Find tensor
    tensor = next((t for t in reader.tensors if t.name == tensor_name), None)
    if tensor is None:
        raise ValueError(f"Tensor {tensor_name} not found")
        
    # 2. Dequantize to NumPy float32
    return gguf.dequantize(tensor.data, tensor.tensor_type)

# Example usage for Llama 3
# w_gate = load_weight("llama3-8b.gguf", "blk.0.ffn_gate.weight")
```

## 6. GGUF File Structure Overview

1. **Header:** Magic bytes (`GGUF`) and version.
2. **Metadata:** Key-Value store containing model config (architecture, head count, etc.).
3. **Tensor Info:** List of all weights, their shapes, and their quantization types (e.g., `Q4_0`, `F16`).
4. **Tensor Data:** The raw byte blocks. Data is aligned (default 32 bytes) to support high-performance `mmap` loading.
