# Research: Multi-Head Attention (MHA) & Grouped-Query Attention (GQA)

This document provides a technical overview of the Attention mechanism used in Transformer architectures like Llama 3 and Mistral, specifically focusing on the inference pipeline.

## 1. Scaled Dot-Product Attention

The core operation of attention is defined by the interaction between Queries ($Q$), Keys ($K$), and Values ($V$).

$$\text{Attention}(Q, K, V) = \text{Softmax}\left(\frac{QK^T}{\sqrt{d_k}} + M\right)V$$

- **$Q \cdot K^T$**: Computes the similarity (alignment) between every query and every key.
- **$\sqrt{d_k}$ Scaling**: Divides by the square root of the head dimension to prevent the dot product from growing too large, which would lead to vanishing gradients in the Softmax.
- **$M$ (Masking)**: Adds a causal mask ($-\infty$ for future tokens) during the pre-fill phase to ensure causality.
- **Softmax**: Normalizes the alignment scores into a probability distribution (attention weights).
- **$\cdot V$**: Computes a weighted sum of the values based on the attention weights.

## 2. Linear Projections

Before the attention mechanism, the input hidden state $x$ is projected into $Q$, $K$, and $V$ spaces using three separate linear layers:

$$Q = x W_Q, \quad K = x W_K, \quad V = x W_V$$

- In Llama 3 8B, $W_Q$ projects to a dimension of $32 \times 128 = 4096$.
- $W_K$ and $W_V$ project to $8 \times 128 = 1024$ (due to GQA).

## 3. Multi-Head Splitting

To allow the model to attend to information from different representation subspaces at different positions, the projected vectors are split into multiple "heads":
- **Query Heads ($n_q$)**: 32 for Llama 3 8B.
- **KV Heads ($n_{kv}$)**: 8 for Llama 3 8B.
- **Head Dimension ($d_h$)**: 128.

## 4. Grouped-Query Attention (GQA)

Llama 3 uses GQA to balance the performance of Multi-Head Attention (MHA) with the memory efficiency of Multi-Query Attention (MQA).

- **Ratio**: Each KV head is shared by multiple Query heads ($n_q / n_{kv}$).
- **Llama 3 8B**: 4 Query heads per 1 KV head.
- **Llama 3 70B**: 8 Query heads per 1 KV head.
- **Benefit**: Significantly reduces the size of the KV cache in memory, which is the primary bottleneck for long-context inference.

## 5. KV Cache for Autoregressive Generation

During token-by-token generation (decoding phase), we only compute the $Q, K, V$ for the *new* token. To avoid recomputing $K$ and $V$ for all previous tokens, we store them in a **KV Cache**.

- **Pre-fill**: Compute and store $K, V$ for the entire prompt.
- **Decode**: For each new token, append its $K, V$ to the cache and perform attention against the full cached sequence.

## 6. Reference NumPy Implementation (GQA + KV Cache)

This implementation integrates our previous RoPE, SoftMax, and MatMul logic.

```python
import numpy as np

def repeat_kv(x, n_rep):
    """Repeat KV heads to match Q heads for GQA."""
    if n_rep == 1:
        return x
    # x shape: (batch, seq_len, n_kv_heads, head_dim)
    return np.repeat(x, n_rep, axis=2)

def grouped_query_attention(q, k, v, mask=None):
    """
    Standard dot-product attention.
    q: (batch, n_q_heads, seq_len_q, head_dim)
    k, v: (batch, n_q_heads, seq_len_kv, head_dim)
    """
    d_k = q.shape[-1]
    # 1. Scaled dot-product: (batch, n_heads, seq_len_q, seq_len_kv)
    scores = np.matmul(q, k.transpose(0, 1, 3, 2)) / np.sqrt(d_k)
    
    if mask is not None:
        scores = scores + mask
        
    # 2. Softmax (using stable trick)
    scores_max = np.max(scores, axis=-1, keepdims=True)
    probs = np.exp(scores - scores_max)
    probs /= np.sum(probs, axis=-1, keepdims=True)
    
    # 3. Weighted sum of values: (batch, n_heads, seq_len_q, head_dim)
    return np.matmul(probs, v)

class Llama3Attention:
    def __init__(self, n_q_heads, n_kv_heads, head_dim):
        self.n_q_heads = n_q_heads
        self.n_kv_heads = n_kv_heads
        self.head_dim = head_dim
        self.n_rep = n_q_heads // n_kv_heads
        
    def forward(self, x, freqs_cis, kv_cache=None, mask=None):
        # 1. Linear Projections (Placeholder for actual weights)
        # q = x @ Wq, k = x @ Wk, v = x @ Wv ...
        
        # Assume q, k, v are already projected and reshaped to:
        # (batch, seq_len, n_heads, head_dim)
        
        # 2. Apply RoPE to Q and K
        # q = apply_rope(q, freqs_cis)
        # k = apply_rope(k, freqs_cis)
        
        # 3. Update KV Cache
        if kv_cache is not None:
            k = np.concatenate([kv_cache['k'], k], axis=1)
            v = np.concatenate([kv_cache['v'], v], axis=1)
            kv_cache['k'], kv_cache['v'] = k, v
            
        # 4. GQA: Repeat K and V heads to match Q
        # (batch, seq_len, n_q_heads, head_dim)
        k_up = repeat_kv(k, self.n_rep)
        v_up = repeat_kv(v, self.n_rep)
        
        # 5. Transpose for attention: (batch, n_heads, seq_len, head_dim)
        q = q.transpose(0, 2, 1, 3)
        k_up = k_up.transpose(0, 2, 1, 3)
        v_up = v_up.transpose(0, 2, 1, 3)
        
        # 6. Attention
        output = grouped_query_attention(q, k_up, v_up, mask)
        
        # 7. Concatenate heads and return: (batch, seq_len, hidden_dim)
        return output.transpose(0, 2, 1, 3).reshape(x.shape[0], x.shape[1], -1)
```
