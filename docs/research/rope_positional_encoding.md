# Research: Rotary Position Encoding (RoPE)

This document details the Rotary Position Encoding (RoPE) mechanism, which is the standard positional encoding method used in modern Transformer architectures like Llama (2, 3, 3.1) and Mistral.

## 1. Mathematical Formulation

RoPE encodes absolute position information by applying a rotation to the Query ($Q$) and Key ($K$) vectors. This rotation is designed such that the inner product between a query and a key depends only on their relative distance.

### Plane Rotations
RoPE pairs adjacent dimensions of a $d$-dimensional head embedding and treats them as coordinates in a 2D plane. For a vector $\mathbf{x}$ at position $m$, the transformation for each pair $(x_{2i}, x_{2i+1})$ is:

$$
\begin{pmatrix} 
x'_{2i} \\ 
x'_{2i+1} 
\end{pmatrix} = 
\begin{pmatrix} 
\cos(m\theta_i) & -\sin(m\theta_i) \\ 
\sin(m\theta_i) & \cos(m\theta_i) 
\end{pmatrix}
\begin{pmatrix} 
x_{2i} \\ 
x_{2i+1} 
\end{pmatrix}
$$

### Complex Representation
In complex space, this is equivalent to representing the pair as $z_i = x_{2i} + ix_{2i+1}$ and multiplying by a unitary complex number:
$$z'_i = z_i \cdot e^{im\theta_i}$$
where $e^{im\theta_i} = \cos(m\theta_i) + i\sin(m\theta_i)$.

## 2. Frequency Computation ($\theta_i$)

The rotation frequencies $\theta_i$ decrease geometrically along the dimension index $i \in \{0, \dots, d/2 - 1\}$:

$$\theta_i = \text{base}^{-2i/d}$$

### Typical Base Values:
- **Llama 2 / Mistral:** `base = 10,000`
- **Llama 3 / 3.1:** `base = 500,000` (Used to support context windows up to 128k tokens).

## 3. Interaction with Attention

The fundamental property of RoPE is that the dot product of two rotated vectors satisfies:
$$\langle \text{RoPE}(\mathbf{q}, m), \text{RoPE}(\mathbf{k}, n) \rangle = g(\mathbf{q}, \mathbf{k}, m-n)$$
This means the attention score $\alpha_{m,n}$ between position $m$ and $n$ is a function of their relative distance $m-n$.

- **Decay:** As relative distance $|m-n|$ increases, the correlation (and thus attention score) naturally decays, mirroring human cognitive patterns where local context is usually more relevant.
- **Independence:** RoPE is applied **after** the linear projections for $Q$ and $K$, but **before** the dot-product attention calculation. It is not applied to the Value ($V$) vectors.

## 4. Reference NumPy Implementation

The following implementation uses NumPy's complex number support for maximum efficiency.

```python
import numpy as np

def precompute_rope_freqs(dim, max_seq_len, theta_base=10000.0):
    """
    Precompute the rotation frequencies.
    dim: head dimension (must be even)
    max_seq_len: max sequence length to cache
    """
    # i ranges from 0 to dim/2 - 1
    i = np.arange(0, dim, 2).astype(np.float32)
    theta = 1.0 / (theta_base ** (i / dim))
    
    # m is the position index
    m = np.arange(max_seq_len)
    
    # outer product gives (seq_len, dim/2) matrix of angles
    freqs = np.outer(m, theta)
    
    # Convert to complex 'cis' format: cos(freqs) + i*sin(freqs)
    return np.exp(1j * freqs)

def apply_rope(x, freqs_cis):
    """
    Apply RoPE to x.
    x: (batch, seq_len, n_heads, head_dim)
    freqs_cis: (seq_len, head_dim // 2) - precomputed complex angles
    """
    # 1. Reshape x to pair dimensions: (..., head_dim//2, 2)
    x_reshaped = x.reshape(*x.shape[:-1], -1, 2)
    
    # 2. View as complex numbers: (..., head_dim//2)
    # Note: Use complex64 if x is float32
    x_complex = x_reshaped[..., 0] + 1j * x_reshaped[..., 1]
    
    # 3. Align freqs_cis for broadcasting: (1, seq_len, 1, head_dim//2)
    freqs_cis = freqs_cis[np.newaxis, :, np.newaxis, :]
    
    # 4. Multiply (rotation in complex plane)
    x_rotated = x_complex * freqs_cis
    
    # 5. Convert back to real pairs
    x_out = np.empty(x_reshaped.shape, dtype=x.dtype)
    x_out[..., 0] = x_rotated.real
    x_out[..., 1] = x_rotated.imag
    
    return x_out.reshape(*x.shape)
```

## 5. Handling Variable Sequence Lengths

RoPE is exceptionally robust for variable sequence lengths:
- **Autoregressive Generation:** In "incremental" mode (one token at a time), the $Q$ and $K$ for the new token are simply rotated using the frequency associated with its current position index $m$.
- **Extrapolation:** Because frequencies are periodic, the model can technically compute rotations for positions $m > \text{training\_length}$. However, performance degrades without specific scaling (like YaRN or Llama 3.1's linear scaling).
- **KV Caching:** Since the rotation depends only on the absolute position $m$ at which the token was generated, the keys in the KV cache do not need to be updated as the sequence grows.
