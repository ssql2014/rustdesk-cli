# vLLM RMSNorm Layer Specification

This document provides the functional specification for the RMSNorm layers implemented in the vLLM project, derived from `vllm/model_executor/layers/layernorm.py`.

## 1. GemmaRMSNorm
Specialized for the Gemma architecture, this layer introduces a weight offset and specific casting logic.

### Parameters
- `hidden_size` (int): Dimension of the hidden state.
- `eps` (float): Numerical stability constant (Default: `1e-6`).

### Initialization
- `self.weight`: Initialized to **zeros** (`torch.zeros`).

### Forward Pass Logic
Signature: `forward(x: Tensor, residual: Optional[Tensor] = None)`

1. **Residual Addition**: If `residual` is provided, $x = x + residual$.
2. **Mean Square Calculation**: $ms = \text{mean}(x^2, \text{dim}=-1, \text{keepdim=True})$.
3. **Normalization**: $\hat{x} = x \cdot \text{rsqrt}(ms + \epsilon)$.
4. **Weight Application**: $y = \hat{x} \cdot (1 + w)$.
5. **Casting**: The result is cast back to the input's original dtype.

**Formula**:
$$y = \left( \frac{x}{\sqrt{\frac{1}{d} \sum x_i^2 + \epsilon}} \right) \odot (1 + w)$$

## 2. RMSNormGated
A unified layer supporting standard RMSNorm, Group RMSNorm, and gating (SiLU).

### Parameters
- `hidden_size` (int): Dimension of the hidden state.
- `eps` (float): Numerical stability constant (Default: `1e-6`).
- `group_size` (int, optional): If provided, performs Group RMSNorm.
- `norm_before_gate` (bool): Order of the gating operation.

### Initialization
- `self.weight`: Initialized to **ones** (`torch.ones`).

### Forward Pass Logic
Signature: `forward(x: Tensor, z: Optional[Tensor] = None)`

#### Case A: `norm_before_gate = False` (Default)
1. **Pre-Gate**: $x' = x \odot \text{SiLU}(z)$.
2. **Variance**: Calculated on $x'$ (using `group_size` if applicable).
3. **Normalize & Scale**: $y = \text{Normalize}(x') \odot w$.

#### Case B: `norm_before_gate = True`
1. **Normalize**: $\hat{x} = \text{Normalize}(x)$.
2. **Post-Gate**: $y = (\hat{x} \odot w) \odot \text{SiLU}(z)$.

### Group Normalization Logic
If `group_size` is $d$, the tensor is viewed as $(\dots, G, d)$ where $G = \text{hidden\_size} / d$. The RMS is calculated independently for each group.

## 3. Implementation Constraints
- **Numerical Stability**: All reductions (sum of squares) and reciprocal square roots **must** be performed in `float32`.
- **Memory Contiguity**: The vLLM implementation includes checks for `is_contiguous()` to enable optimized kernel dispatch.
- **Epsilon Usage**: Epsilon is strictly inside the square root: $1/\sqrt{var + \epsilon}$.
