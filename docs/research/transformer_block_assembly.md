# Transformer Block Assembly: Llama 3 Style

This document describes how to assemble individual neural operators into a functional Llama-style Transformer block and the broader inference pipeline.

## 1. Block Architecture

The Llama 3 transformer block uses a "pre-normalization" architecture with residual connections. A single block consists of two main sub-layers: the Attention mechanism and the Feed-Forward Network (MLP).

### Step-by-Step Forward Pass:
1.  **Input ($x$):** The hidden state from the previous layer.
2.  **Attention Sub-layer:**
    - `norm_x = RMSNorm(x)`
    - `attn_out = MultiHeadAttention(norm_x)` (includes Linear Projections and RoPE)
    - `x = x + attn_out` (Residual Addition)
3.  **MLP Sub-layer:**
    - `norm_x = RMSNorm(x)`
    - `mlp_out = SwiGLU_MLP(norm_x)` (MatMul -> SiLU Gate -> Element-wise Multi -> MatMul)
    - `x = x + mlp_out` (Residual Addition)
4.  **Output:** The updated hidden state passed to the next block.

## 2. Data Flow & Shapes (Llama 3 8B)

| Stage | Input Shape | Operator | Output Shape |
| :--- | :--- | :--- | :--- |
| **Start** | `(B, S, 4096)` | - | `(B, S, 4096)` |
| **Attention Norm** | `(B, S, 4096)` | RMSNorm | `(B, S, 4096)` |
| **Q Projection** | `(B, S, 4096)` | Linear ($W_q$) | `(B, S, 32, 128)` |
| **KV Projection** | `(B, S, 4096)` | Linear ($W_k, W_v$) | `(B, S, 8, 128)` |
| **Attention** | `(B, 32, S, 128)` | GQA + SoftMax | `(B, S, 4096)` |
| **MLP Norm** | `(B, S, 4096)` | RMSNorm | `(B, S, 4096)` |
| **Gate/Up Proj** | `(B, S, 4096)` | Linear ($W_g, W_u$) | `(B, S, 14336)` |
| **Activation** | `(B, S, 14336)` | SiLU(gate) * up | `(B, S, 14336)` |
| **Down Proj** | `(B, S, 14336)` | Linear ($W_d$) | `(B, S, 4096)` |

*(B = Batch Size, S = Sequence Length)*

## 3. NumPy Reference Implementation

This class chains our researched operators into a single block.

```python
import numpy as np

class Llama3Block:
    def __init__(self, layer_id, args):
        self.layer_id = layer_id
        # Weights would be loaded here: 
        # w_q, w_k, w_v, w_o, w_gate, w_up, w_down, rms_attn, rms_ffn
        self.weights = {} 

    def forward(self, x, freqs_cis, mask=None, kv_cache=None):
        # --- 1. Attention ---
        # Residual 1
        h = x + self.attention_forward(rms_norm(x, self.weights['rms_attn']), 
                                       freqs_cis, mask, kv_cache)
        
        # --- 2. MLP (SwiGLU) ---
        # Residual 2
        out = h + self.mlp_forward(rms_norm(h, self.weights['rms_ffn']))
        
        return out

    def attention_forward(self, x, freqs_cis, mask, kv_cache):
        # Q, K, V Projections
        q = matmul(x, self.weights['w_q']) # (B, S, n_q * d_h)
        k = matmul(x, self.weights['w_k']) # (B, S, n_kv * d_h)
        v = matmul(x, self.weights['w_v']) # (B, S, n_kv * d_h)
        
        # Reshape for multi-head
        q = q.reshape(B, S, n_q, d_h)
        k = k.reshape(B, S, n_kv, d_h)
        v = v.reshape(B, S, n_kv, d_h)
        
        # Apply RoPE
        q = apply_rope(q, freqs_cis)
        k = apply_rope(k, freqs_cis)
        
        # Core Attention (GQA)
        attn_out = grouped_query_attention(q, k, v, mask, kv_cache)
        
        # Output Projection
        return matmul(attn_out, self.weights['w_o'])

    def mlp_forward(self, x):
        # SwiGLU: (SiLU(xW_g) * xW_u)W_d
        gate = matmul(x, self.weights['w_gate'])
        up = matmul(x, self.weights['w_up'])
        
        activated = silu(gate) * up
        return matmul(activated, self.weights['w_down'])
```

## 4. Layer Stacking

For Llama 3 8B, the model consists of **32 identical blocks** stacked sequentially.
- The output of `block[i]` becomes the input to `block[i+1]`.
- The residual connections within each block ensure that the gradient (during training) or information (during inference) can flow through the deep stack without vanishing.

## 5. Full Inference Pipeline

The complete end-to-end forward pass follows this sequence:

1.  **Embedding:** `token_ids` $\rightarrow$ `(B, S, 4096)`
2.  **Transformer Stack:** Pass through 32 `Llama3Block` instances.
3.  **Final Norm:** Apply one last `RMSNorm` to the output of the final block.
4.  **Output Head (lm_head):** Linear projection from `hidden_dim` (4096) $\rightarrow$ `vocab_size` (128,256). Result is "logits".
5.  **SoftMax:** (Optional for sampling) Convert logits to probabilities.
6.  **Token Selection:** Choose the next token ID (ArgMax or Top-P/K sampling).
7.  **Loop:** Feed the new token back into the pipeline (using the KV cache for efficiency).
