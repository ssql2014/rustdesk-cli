# Research: SoftMax Operator

This document provides a technical overview of the SoftMax operator, its numerical stability implementation, and its critical role in Transformer-based attention mechanisms (Llama, Mistral).

## 1. Mathematical Formulation

SoftMax converts a vector of raw scores (logits) into a probability distribution where all elements are in the range $(0, 1)$ and sum to $1.0$.

### Standard Definition
For an input vector $\mathbf{x}$ of dimension $d$:
$$\text{Softmax}(x_i) = \frac{e^{x_i}}{\sum_{j=1}^d e^{x_j}}$$

### Numerical Stability (The Max Subtraction Trick)
Directly computing $e^{x_i}$ is prone to **overflow** if $x_i$ is large (e.g., $x_i > 88$ for `float32`). To prevent this, we use the property that $\text{Softmax}(\mathbf{x}) = \text{Softmax}(\mathbf{x} - C)$ for any constant $C$. By choosing $C = \max(\mathbf{x})$, we ensure all exponents are $\le 0$.

$$\text{Softmax}(x_i) = \frac{e^{x_i - \max(\mathbf{x})}}{\sum_{j=1}^d e^{x_j - \max(\mathbf{x})}}$$

This ensures the largest value becomes $e^0 = 1$ and all other values are between $0$ and $1$, eliminating the risk of `inf` or `NaN`.

## 2. Role in Scaled Dot-Product Attention

In Transformer architectures, SoftMax is used to normalize the attention scores:
$$\text{Attention}(Q, K, V) = \text{Softmax}\left(\frac{QK^T}{\sqrt{d_k}} + M\right)V$$

- **Scaling:** The scores are divided by $\sqrt{d_k}$ to prevent the dot product from growing too large in magnitude, which would push the SoftMax into regions with extremely small gradients.
- **Weights:** The SoftMax output represents the "weight" or "importance" that each Query token assigns to each Key token.

## 3. Causal Masking Interaction

For autoregressive generation (predicting the next token), a token at position $i$ must not attend to tokens at positions $j > i$.

- **Masking:** Before applying SoftMax, we add a mask matrix $M$ where $M_{ij} = -\infty$ (or a large negative constant like $-1e9$) for $j > i$, and $0$ otherwise.
- **Result:** Since $e^{-\infty} = 0$, the SoftMax probabilities for future tokens become exactly $0$, effectively "masking" them out of the weighted sum of Value vectors.

## 4. Reference NumPy Implementation

The following implementation is designed for deployment to the remote peer `308235080`.

```python
import numpy as np

def softmax(x, axis=-1, mask=None):
    """
    Stable SoftMax implementation with optional masking.
    x: Input array
    axis: Dimension along which to compute SoftMax
    mask: Boolean mask (True for positions to keep, False to mask out)
    """
    if mask is not None:
        # Fill masked positions with a large negative constant
        # Use -1e9 for compatibility with float16/float32
        x = np.where(mask, x, -1e9)
    
    # 1. Stability trick: subtract max
    # Keepdims is essential for correct broadcasting
    x_max = np.max(x, axis=axis, keepdims=True)
    exp_x = np.exp(x - x_max)
    
    # 2. Sum and normalize
    sum_exp = np.sum(exp_x, axis=axis, keepdims=True)
    return exp_x / sum_exp

def verify_softmax():
    # Test 1: Basic probability property
    x = np.array([1.0, 2.0, 3.0])
    out = softmax(x)
    assert np.allclose(np.sum(out), 1.0), "Sum should be 1.0"
    assert np.all(out > 0), "Probabilities should be positive"
    
    # Test 2: Numerical stability
    x_large = np.array([1000.0, 1001.0, 1002.0])
    out_large = softmax(x_large)
    assert not np.any(np.isnan(out_large)), "Should not produce NaN"
    assert np.argmax(out_large) == 2, "Max index should be preserved"
    
    # Test 3: Masking
    x_mask = np.array([10.0, 10.0, 10.0])
    mask = np.array([True, True, False])
    out_mask = softmax(x_mask, mask=mask)
    assert out_mask[2] < 1e-8, "Masked position should be ~0"
    assert np.allclose(out_mask[:2], 0.5), "Remaining positions should split probability"
    
    print("SoftMax verification successful.")

if __name__ == "__main__":
    verify_softmax()
```

## 5. Performance Considerations

- **Memory Usage:** SoftMax requires materializing the full $N \times N$ attention matrix (where $N$ is sequence length). For $N=32768$, this is $10^9$ elements (~4GB in `float32`), which can exceed memory on many nodes.
- **Flash Attention:** In production systems, SoftMax is often "fused" into the attention kernel (Flash Attention) to avoid materializing the full matrix, reducing memory complexity from $O(N^2)$ to $O(N)$.
- **Axis Selection:** In NumPy, ensure `axis=-1` is used to normalize across the sequence length dimension (keys) rather than the batch or head dimensions.
