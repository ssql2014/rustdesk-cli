# Research: KV Cache Implementation for Efficient Inference

This document details the KV (Key-Value) Cache mechanism used to optimize autoregressive token generation in Transformer models like Llama 3.

## 1. Why KV Cache is Necessary

In autoregressive generation, each new token depends on all previously generated tokens.
- **Standard Forward Pass:** A Transformer computes attention by multiplying Queries ($Q$) with all previous Keys ($K$) and then by Values ($V$).
- **The Redundancy:** Since previous tokens do not change during the generation of the next token, their $K$ and $V$ vectors are also constant.
- **The Solution:** The **KV Cache** stores the $K$ and $V$ vectors for all past tokens. At each step, we only compute $Q, K, V$ for the *single new token*, append the new $K, V$ to the cache, and perform attention against the full cached history. This reduces the computational complexity per step from $O(N^2)$ to $O(N)$, where $N$ is the sequence length.

## 2. Architecture and Memory (Llama 3 8B)

For Llama 3 8B, the KV cache dimensions are determined by the Grouped-Query Attention (GQA) configuration.

### Dimensions per Layer:
- **Number of KV Heads ($n_{kv}$):** 8 (Shared by 32 Query heads)
- **Head Dimension ($d_h$):** 128
- **Cache Shape:** `[batch, n_kv_heads, seq_len, head_dim]`

### Memory Calculation (FP16/BF16):
- **Elements per token per layer:** $2 \times 8 \text{ heads} \times 128 \text{ dim} = 2,048$ elements.
- **Memory per token (all 32 layers):** $2,048 \times 32 \times 2 \text{ bytes} = 131,072 \text{ bytes} \approx \mathbf{128 \text{ KB}}$.
- **Memory for 8,192 Context:** $\approx \mathbf{1 \text{ GB}}$.

## 3. Allocation Strategies

### Static Pre-allocation (Official Llama)
The official Meta reference implementation pre-allocates the entire cache buffer at initialization.
- **Pros:** Simple to implement; memory is contiguous, allowing for high-performance BLAS operations.
- **Cons:** High VRAM waste (reservations for unused context) and internal fragmentation.

### Dynamic PagedAttention (vLLM)
Production engines like vLLM use a virtual memory approach.
- **Logic:** Memory is allocated in small, non-contiguous "pages" (blocks of 16 tokens).
- **Pros:** Near-zero memory waste; supports prefix caching (sharing the same physical blocks for identical prompts across multiple requests).
- **Cons:** Significant implementation complexity; requires custom CUDA kernels to handle non-contiguous memory access.

## 4. Cache Management and Eviction

- **Full Context:** The cache grows until it hits `max_seq_len`. Generation then stops.
- **Sliding Window Attention (SWA):** Used in models like Mistral. The cache only keeps the most recent $W$ tokens (e.g., 4096). This keeps memory usage constant but limits long-term dependencies.
- **Quantization:** High-performance systems often quantize the KV cache to **INT8** or **INT4** to double or quadruple the maximum supported context length on the same hardware.

## 5. NumPy Reference Implementation (Single Decode Step)

This implementation shows how to update a pre-allocated contiguous cache during one step of autoregressive decoding.

```python
import numpy as np

class KVCache:
    def __init__(self, n_layers, batch_size, n_kv_heads, max_seq_len, head_dim):
        self.max_seq_len = max_seq_len
        # Allocate contiguous buffers for Keys and Values
        # Shape: (layers, batch, heads, max_len, head_dim)
        self.k = np.zeros((n_layers, batch_size, n_kv_heads, max_seq_len, head_dim), dtype=np.float32)
        self.v = np.zeros((n_layers, batch_size, n_kv_heads, max_seq_len, head_dim), dtype=np.float32)
        self.current_pos = 0

    def update(self, layer_idx, new_k, new_v):
        """
        new_k, new_v shape: (batch, n_kv_heads, 1, head_dim)
        """
        start = self.current_pos
        end = start + 1
        
        # Insert new token's K, V into the pre-allocated buffer
        self.k[layer_idx, :, :, start:end, :] = new_k
        self.v[layer_idx, :, :, start:end, :] = new_v
        
        # Return the full prefix up to the new token
        return self.k[layer_idx, :, :, :end, :], self.v[layer_idx, :, :, :end, :]

    def increment_pos(self):
        self.current_pos += 1

# --- Example Usage in a Decode Loop ---
def decode_step(model, token_id, cache):
    # 1. Forward pass for ONLY the new token
    # q, k, v = model.project(token_id)
    
    # 2. Update cache for each layer
    for i in range(32):
        # full_k, full_v = cache.update(i, k[i], v[i])
        # attn_out = attention(q[i], full_k, full_v)
        pass
    
    cache.increment_pos()
```

## 6. Summary for Remote Inference

For our remote deployment on peer `308235080`:
1.  **Strategy:** Use **Contiguous Pre-allocation**. Given the limited scope, PagedAttention is too complex for a pure Python/NumPy deployment.
2.  **Optimization:** If RAM becomes an issue, implement a **Sliding Window** to cap the memory usage at a fixed number of tokens (e.g., 2048 tokens $\approx 256$ MB).
