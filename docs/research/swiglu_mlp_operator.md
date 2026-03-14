# Research: SwiGLU MLP Operator

This document provides a technical overview of the SwiGLU MLP (Multi-Layer Perceptron) used in Llama 3 architectures and a reference implementation for remote deployment.

## 1. Mathematical Formulation

The SwiGLU MLP is a variation of the Gated Linear Unit (GLU). In Llama 3, it replaces the standard FFN (Feed-Forward Network) to provide better expressivity and convergence.

The computation for an input vector $x$ is:
$$\text{SwiGLU}(x) = (\text{SiLU}(x W_{gate}) \otimes (x W_{up})) W_{down}$$

- **SiLU (Sigmoid Linear Unit):** $\text{SiLU}(z) = z \cdot \sigma(z) = \frac{z}{1 + e^{-z}}$.
- **$\otimes$**: Hadamard (element-wise) product.
- **$W_{gate}$**: Gate projection matrix.
- **$W_{up}$**: Up projection matrix (transformation branch).
- **$W_{down}$**: Down projection matrix (reduces back to hidden dimension).

## 2. Dimensions (Llama 3 8B)

For Llama 3 8B, the dimensions are significantly larger than Llama 2 to increase capacity.

- **Hidden Dimension ($d_{model}$):** 4,096
- **Intermediate Dimension ($d_{ff}$):** 14,336
- **Weights:**
    - $W_{gate} \in \mathbb{R}^{4096 \times 14336}$
    - $W_{up} \in \mathbb{R}^{4096 \times 14336}$
    - $W_{down} \in \mathbb{R}^{14336 \times 4096}$

## 3. Memory Requirements

The MLP is the most memory-intensive part of the transformer block.

- **Parameters per Layer:** $(4096 \times 14336) \times 3 \approx 176.16 \text{M}$.
- **Total Parameters (32 Layers):** $176.16 \text{M} \times 32 \approx 5.64 \text{B}$.
- **RAM (Weights only):**
    - **FP32:** ~22.5 GB
    - **FP16 / BF16:** ~11.3 GB
    - **INT4 (Quantized):** ~3.2 GB

## 4. Implementation Details: Weight Fusion

In optimized inference engines, $W_{gate}$ and $W_{up}$ are often **fused** into a single matrix of shape $[4096, 28672]$.
- **Benefit:** Allows computing both projections in a single GEMM call, improving GPU/CPU throughput by reducing kernel launch overhead and improving memory locality.
- **Computation:** The output of $(x @ W_{fused})$ is split into two halves ($gate$ and $up$), which are then activated and multiplied.

## 5. NumPy Reference Implementation

This implementation is designed for deployment to the remote peer `308235080`.

```python
import numpy as np

def silu(x):
    """SiLU activation function."""
    return x * (1.0 / (1.0 + np.exp(-x)))

def swiglu_mlp(x, w_gate, w_up, w_down):
    """
    Standard SwiGLU MLP implementation.
    x: Input [..., 4096]
    w_gate: [4096, 14336]
    w_up: [4096, 14336]
    w_down: [14336, 4096]
    """
    # 1. Gate branch: SiLU(x @ W_gate)
    gate = silu(np.matmul(x, w_gate))
    
    # 2. Up branch: x @ W_up
    up = np.matmul(x, w_up)
    
    # 3. Fuse branches: gate * up
    intermediate = gate * up
    
    # 4. Down projection: intermediate @ W_down
    return np.matmul(intermediate, w_down)

def swiglu_mlp_fused(x, w_fused, w_down):
    """
    Fused version using a single [4096, 28672] projection.
    """
    # 1. Combined projection
    projected = np.matmul(x, w_fused)
    
    # 2. Split into gate and up
    gate, up = np.split(projected, 2, axis=-1)
    
    # 3. Activate and multiply
    intermediate = silu(gate) * up
    
    # 4. Down projection
    return np.matmul(intermediate, w_down)
```

## 6. Numerical Stability

- **Quadratic Growth:** Unlike ReLU (linear), SwiGLU can exhibit $O(x^2)$ growth if $W_{gate}$ and $W_{up}$ become highly aligned. This makes it more prone to **overflow** in low-precision formats (FP8).
- **Gradient Flow:** SwiGLU is superior to ReLU for preventing "dying neurons" because the SiLU activation is smooth and provides non-zero gradients for negative inputs.
- **Recommendation:** When deploying in FP16 or BF16, no special stabilization is typically required.
