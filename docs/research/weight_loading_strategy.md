# Research: Remote Weight Loading Strategy

This document details the strategy for loading and managing Llama 3 8B weights (~4.65GB in Q4_K_M) on a remote peer over a RustDesk connection.

## 1. Loading Strategy: Memory Mapping (`mmap`)

For remote inference on potentially memory-constrained peers, **Memory Mapping (`mmap`)** is the superior strategy compared to full RAM loading.

- **Mechanism:** The `gguf-py` reader (and `llama.cpp`) uses `mmap` by default to map the model file into the process's virtual address space.
- **Benefits for Remote Peer:**
    - **Near-Instant Startup:** The inference process starts in milliseconds because no data is actually read yet.
    - **On-Demand Loading:** Only the weights actually used in the forward pass are paged into physical RAM by the OS.
    - **Cache Management:** If the peer runs low on RAM, the OS can transparently evict weight pages and reload them from disk later, preventing Out-Of-Memory (OOM) crashes.
- **Requirement:** Requires the peer to have a relatively fast disk (SSD) to avoid "page fault" stuttering during the first inference pass.

## 2. Bandwidth and Transfer Estimates

The primary bottleneck for remote deployment is the initial push of the 4.65GB model file over the RustDesk relay.

| Speed | Effective Bandwidth | Estimated Transfer Time |
| :--- | :--- | :--- |
| **10 Mbps** | ~1.25 MB/s | **~1 hour 2 mins** |
| **25 Mbps** | ~3.12 MB/s | **~25 minutes** |
| **50 Mbps** | ~6.25 MB/s | **~12 minutes** |
| **100 Mbps** | ~12.5 MB/s | **~6 minutes** |

**Recommendation:** Implement transfer resuming in `rustdesk-cli push` (using the `FileTransferDigest` protocol) to ensure that interupted 1-hour transfers don't restart from zero.

## 3. Weight Sharding and Incremental Deployment

GGUF supports **sharding** (splitting the model into multiple `.gguf` files).

- **Can we shard?** Yes. Using `llama-gguf-split`, we can break the 4.65GB model into 500MB shards.
- **Why shard?**
    - **Resilience:** If one shard fails to transfer, only that 500MB needs to be re-sent.
    - **Verification:** We can verify each shard individually after transfer.
- **Incremental Loading:** `gguf-py` can load tensors from specific shards. This allows us to start deploying the pipeline logic and early layers while later shards are still transferring.

## 4. Pipeline Integration (`llama3_pipeline_op.py`)

Our remote pipeline operator must transition from random tensors to GGUF loading.

```python
import gguf
import numpy as np

class RemoteLlama3:
    def __init__(self, gguf_path):
        self.reader = gguf.GGUFReader(gguf_path)
        self.tensors = {t.name: t for t in self.reader.tensors}

    def load_weight(self, name):
        """Loads and dequantizes a tensor on-the-fly."""
        tensor = self.tensors.get(name)
        if tensor is None:
            raise ValueError(f"Weight {name} not found")
        # Dequantize to float32 for computation
        return gguf.dequantize(tensor.data, tensor.tensor_type)

# Usage in pipeline
# self.w_gate = model.load_weight("blk.0.ffn_gate.weight")
```

## 5. Approach Comparison

| Approach | Pros | Cons |
| :--- | :--- | :--- |
| **Push Full GGUF** | Simplest logic; native `llama.cpp` compatibility. | High risk of failure on slow links; high initial wait. |
| **Push Shards** | More resilient; supports incremental deployment. | Slightly more complex file management on peer. |
| **Lazy-Load (Shared)**| Zero push time. | Requires high-speed shared storage (not applicable for RustDesk peers). |

**Final Recommendation:** Use **Sharded GGUF (500MB shards)** combined with **`mmap` loading**. This provides the best balance of deployment resilience and runtime memory efficiency for remote AI agents.
