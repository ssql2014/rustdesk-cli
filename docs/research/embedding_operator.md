# Research: Embedding Operator

This document provides a technical overview of the Embedding operator, its dimensions in modern Transformer architectures (Llama, Mistral), and its implementation for inference.

## 1. Mechanism: Embedding Lookup

The Embedding operator acts as a lookup table that maps discrete token IDs (integers) to continuous dense vectors.

- **Operation:** Given an input tensor of token IDs $\mathbf{t} \in \mathbb{Z}^N$ and a weight matrix $\mathbf{W} \in \mathbb{R}^{V \times D}$ (where $V$ is vocabulary size and $D$ is hidden dimension), the output is $\mathbf{E} = \mathbf{W}[\mathbf{t}]$.
- **Computational Nature:** Logically, it is equivalent to a Matrix Multiplication with a one-hot encoded vector ($\text{OneHot}(\mathbf{t}) \cdot \mathbf{W}$), but in practice, it is implemented as a simple memory indexing operation for efficiency.

## 2. Table Dimensions (Llama & Mistral)

The vocabulary size ($V$) has grown significantly in recent models to improve tokenization efficiency.

| Model | Vocab Size ($V$) | Hidden Dim ($D$) | Total Parameters |
| :--- | :--- | :--- | :--- |
| **Llama 2 (7B)** | 32,000 | 4,096 | ~131 Million |
| **Mistral (7B)** | 32,000 | 4,096 | ~131 Million |
| **Llama 3 (8B)** | 128,256 | 4,096 | ~525 Million |
| **Llama 3 (70B)** | 128,256 | 8,192 | ~1.05 Billion |

## 3. Weight Tying (Tie-Word Embeddings)

Weight tying is the practice of using the same weight matrix for both the input embeddings and the final output projection (`lm_head`).

- **Traditional (GPT-2):** Often used weight tying to save memory.
- **Llama 2 / Llama 3 (8B/70B):** **Do not use weight tying.** The input embedding matrix and the output `lm_head` matrix are separate, optimized parameters.
- **Llama 3.2 (1B/3B):** Use weight tying to minimize the footprint for mobile/edge deployment.

## 4. Reference NumPy Implementation

The following implementation is designed for deployment to the remote peer `308235080`.

```python
import numpy as np

def embedding_lookup(tokens, weights):
    """
    Performs embedding lookup using NumPy indexing.
    tokens: Array of integer token IDs, shape (batch, seq_len)
    weights: Embedding table, shape (vocab_size, hidden_dim)
    """
    # NumPy handles integer indexing across any number of dimensions
    return weights[tokens]

def verify_embedding():
    vocab_size = 1000
    hidden_dim = 128
    seq_len = 10
    
    # Initialize dummy weights
    weights = np.random.randn(vocab_size, hidden_dim).astype(np.float32)
    
    # Input tokens
    tokens = np.array([0, 5, 999, 42])
    
    # Lookup
    output = embedding_lookup(tokens, weights)
    
    # Verification
    assert output.shape == (4, 128), f"Shape mismatch: {output.shape}"
    assert np.array_equal(output[0], weights[0]), "Index 0 lookup failed"
    assert np.array_equal(output[2], weights[999]), "Index 999 lookup failed"
    
    print("Embedding verification successful.")

if __name__ == "__main__":
    verify_embedding()
```

## 5. Memory and Performance Considerations

### Memory Footprint
Large vocabularies have a significant impact on GPU/RAM requirements. For Llama 3 (128k vocab, 4096 hidden dim):
- **FP32 (4 bytes):** $128,256 \times 4,096 \times 4 \approx 2.1$ GB
- **FP16/BF16 (2 bytes):** $128,256 \times 4,096 \times 2 \approx 1.05$ GB
- **INT4 Quantized (0.5 bytes):** $128,256 \times 4,096 \times 0.5 \approx 262$ MB

*Note: Since Llama 3 doesn't tie weights, you must double these figures to account for the `lm_head` at the end of the model.*

### Memory Locality
Embedding lookup is a **memory-bound** operation. Performance is determined by the speed of random memory access rather than arithmetic throughput. 
- In distributed inference (Multi-GPU), the embedding table is often the first thing to be sharded (Tensor Parallelism).
- In CPU inference, ensuring the embedding table stays in cache (if small) or uses fast RAM is critical.
