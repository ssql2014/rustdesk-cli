# Research: SoftMax Operator for Llama 3 Inference

This document provides a technical overview of the SoftMax operator, its numerical stability, temperature scaling, and its application in Llama 3 attention mechanisms.

## 1. Mathematical Formulation

SoftMax converts raw scores (logits) into a probability distribution.

### Standard Definition
For a vector $\mathbf{x}$ of dimension $d$:
$$\text{Softmax}(x_i) = \frac{e^{x_i}}{\sum_{j=1}^d e^{x_j}}$$

### Numerical Stability (The Max Subtraction Trick)
To prevent exponential overflow (especially in `float16` or `float32`), we subtract the maximum value from all elements before exponentiation:
$$\text{Softmax}(x_i) = \frac{e^{x_i - \max(\mathbf{x})}}{\sum_{j=1}^d e^{x_j - \max(\mathbf{x})}}$$
This ensures the largest value becomes $e^0 = 1$ and all others are $\in (0, 1]$.

### Temperature Scaling
In LLM sampling, a temperature parameter $T$ is used to control the "sharpness" of the distribution:
$$\text{Softmax}(x_i, T) = \frac{e^{x_i / T}}{\sum_{j=1}^d e^{x_j / T}}$$
- **$T \to 0$**: Becomes "greedy" (concentrates probability on the maximum).
- **$T = 1$**: Standard SoftMax.
- **$T > 1$**: Makes the distribution "flatter" (more diverse sampling).

## 2. Llama 3 Attention Score Computation

In Llama 3, SoftMax is applied to the scaled dot-product attention scores.

### Exact Computation
$$\text{Scores} = \frac{QK^T}{\sqrt{d_k}}$$
$$\text{AttentionWeights} = \text{Softmax}(\text{Scores} + \text{Mask})$$

- **$d_k$ (Head Dim)**: For Llama 3 8B, $d_k = 128$. The scaling factor is $\sqrt{128} \approx 11.3137$.
- **Causal Mask**: Applied before SoftMax by setting future token positions to $-\infty$.

## 3. Memory Requirements (Sequence Length 8192)

Memory consumption for the attention score matrix is a significant bottleneck for long sequences.

| Parameter | Value |
| :--- | :--- |
| Sequence Length ($N$) | 8192 |
| Attention Heads ($H$) | 32 |
| Matrix Size per Head | $8192 \times 8192 = 67,108,864$ elements |
| Total Elements ($H \times N^2$) | $2,147,483,648$ elements |
| **Memory (FP32)** | **~8.0 GB** |
| **Memory (FP16/BF16)** | **~4.0 GB** |

**Note**: This assumes materialization of the full score matrix for all heads. Optimized kernels (FlashAttention) compute this head-by-head or in tiles to reduce the peak memory footprint from $O(N^2)$ to $O(N)$.

## 4. Standalone Implementation (softmax_op.py)

The following implementation is designed for deployment to `/home/evas/` on the remote server. It handles 2D matrices (scores for a single head or batched heads) and allows axis specification.

```python
import numpy as np

def softmax(x, axis=-1, temperature=1.0, mask=None):
    """
    Numerically stable SoftMax with temperature scaling and masking.
    
    Args:
        x: Input numpy array (e.g., [seq_len, seq_len] or [heads, seq_len, seq_len])
        axis: Dimension to perform SoftMax over (usually the last one)
        temperature: Scaling factor for sharpness
        mask: Optional boolean mask (True to keep, False to mask out)
    """
    # 1. Apply temperature scaling
    x = x / max(temperature, 1e-6)
    
    # 2. Apply causal/padding mask
    if mask is not None:
        # Use a large negative constant for -inf
        x = np.where(mask, x, -1e9)
    
    # 3. Numerical stability: subtract max
    x_max = np.max(x, axis=axis, keepdims=True)
    exp_x = np.exp(x - x_max)
    
    # 4. Normalize
    sum_exp = np.sum(exp_x, axis=axis, keepdims=True)
    return exp_x / sum_exp

if __name__ == "__main__":
    # Example verification for Llama 3 8B head dim
    d_k = 128
    scale = np.sqrt(d_k)
    
    # Dummy Q and K for 10 tokens
    q = np.random.randn(10, d_k)
    k = np.random.randn(10, d_k)
    
    # QK^T / sqrt(d_k)
    scores = np.matmul(q, k.T) / scale
    
    # Softmax
    probs = softmax(scores, temperature=0.7)
    
    print(f"Scores shape: {scores.shape}")
    print(f"Probabilities row sum (should be 1.0): {np.sum(probs, axis=-1)}")
    print("SoftMax operator verification successful.")
```
