# Research: Multi-Head Attention (MHA) with Grouped Query Attention (GQA)

This document details the Multi-Head Attention mechanism used in Llama 3 8B, specifically focusing on the implementation of Grouped Query Attention (GQA) for efficient inference.

## 1. Grouped Query Attention (GQA)

Llama 3 8B uses GQA to reduce the memory footprint and bandwidth requirements of the KV cache. While standard Multi-Head Attention (MHA) uses an equal number of Query (Q), Key (K), and Value (V) heads, GQA groups multiple Q heads to share a single KV head.

- **Query Heads ($n_q$):** 32
- **KV Heads ($n_{kv}$):** 8
- **Group Size:** $n_q / n_{kv} = 4$.
- **Mechanism:** Every 4 Query heads share the same Key and Value head. During computation, the K and V tensors are "repeated" or broadcasted across the 4 heads in their group.

## 2. Complete Attention Computation

The attention pass for a single transformer block follows these steps:

1.  **Linear Projections:**
    - $Q = x \cdot W_q$
    - $K = x \cdot W_k$
    - $V = x \cdot W_v$
2.  **Reshape & RoPE:**
    - Reshape $Q, K, V$ to split into heads: `[seq_len, n_heads, head_dim]`.
    - Apply **Rotary Position Encoding (RoPE)** to $Q$ and $K$.
3.  **KV Cache Update:**
    - Append the new $K$ and $V$ vectors to the session's KV cache.
4.  **GQA Expansion:**
    - Expand $K$ and $V$ from 8 heads to 32 heads by repeating each KV head 4 times to match the Q head groups.
5.  **Score Calculation:**
    - $\text{Scores} = \frac{Q \cdot K^T}{\sqrt{d_k}}$ where $d_k = 128$.
6.  **Causal Masking:**
    - Apply a triangular mask to ensure tokens only attend to the past.
7.  **SoftMax:**
    - Apply numerically stable **SoftMax** to the scores.
8.  **Context Aggregation:**
    - $\text{AttentionOut} = \text{SoftMax}(\text{Scores}) \cdot V$.
9.  **Output Projection:**
    - Concatenate all 32 heads and project back to hidden dimension: $x_{out} = \text{AttentionOut} \cdot W_o$.

## 3. Weight Shapes (Llama 3 8B)

| Weight | Shape | Description |
| :--- | :--- | :--- |
| **$W_q$** | `[4096, 4096]` | 32 heads * 128 dim |
| **$W_k$** | `[4096, 1024]` | 8 heads * 128 dim |
| **$W_v$** | `[4096, 1024]` | 8 heads * 128 dim |
| **$W_o$** | `[4096, 4096]` | Output projection |

## 4. Operator Integration

The attention block serves as the orchestrator for several other operators:
- **RoPE**: Applied immediately after projection.
- **KV Cache**: Handles the storage of $K, V$ across steps.
- **SoftMax**: Normalizes the attention scores.
- **MatMul**: Used for all 4 linear projections ($W_q, W_k, W_v, W_o$) and the core $Q \cdot K^T$ and $\text{Attn} \cdot V$ operations.

## 5. Performance and Memory

By using GQA (8 KV heads instead of 32), Llama 3 8B reduces the KV cache size by **75%** compared to a model with the same head count using standard MHA. For a sequence length of 8192, this saves approximately 3 GB of VRAM per user session.
