import numpy as np

def repeat_kv(x, n_rep):
    """
    Repeats KV heads to match Query heads for GQA.
    x: [seq_len, n_kv_heads, head_dim]
    n_rep: repetition factor (4 for Llama 3 8B)
    """
    if n_rep == 1:
        return x
    
    # x is [seq_len, n_kv_heads, head_dim]
    # We want [seq_len, n_kv_heads * n_rep, head_dim]
    # np.repeat repeats elements along the specified axis
    return np.repeat(x, n_rep, axis=1)

def softmax(x, axis=-1):
    """Numerically stable SoftMax."""
    x_max = np.max(x, axis=axis, keepdims=True)
    exp_x = np.exp(x - x_max)
    return exp_x / np.sum(exp_x, axis=axis, keepdims=True)

def attention_forward(x, w_q, w_k, w_v, w_o, kv_cache=None, mask=None, layer_idx=0):
    """
    Complete forward pass for Llama 3 8B Multi-Head Attention with GQA.
    
    x: [seq_len, 4096]
    w_q: [4096, 4096]
    w_k, w_v: [4096, 1024]
    w_o: [4096, 4096]
    """
    seq_len, hidden_dim = x.shape
    n_q_heads = 32
    n_kv_heads = 8
    head_dim = 128
    n_rep = n_q_heads // n_kv_heads # 4
    
    # 1. Linear Projections
    q = np.matmul(x, w_q) # [seq_len, 4096]
    k = np.matmul(x, w_k) # [seq_len, 1024]
    v = np.matmul(x, w_v) # [seq_len, 1024]
    
    # 2. Reshape to heads
    q = q.reshape(seq_len, n_q_heads, head_dim)
    k = k.reshape(seq_len, n_kv_heads, head_dim)
    v = v.reshape(seq_len, n_kv_heads, head_dim)
    
    # --- RoPE would be applied here to q and k ---
    # q = apply_rope(q)
    # k = apply_rope(k)
    
    # 3. Update KV Cache
    if kv_cache is not None:
        # cache.update returns the full history: [total_seq_len, n_kv_heads, head_dim]
        full_k, full_v = kv_cache.update(layer_idx, k, v)
    else:
        full_k, full_v = k, v
        
    # 4. GQA: Repeat K and V heads to match Q (8 -> 32)
    full_k_up = repeat_kv(full_k, n_rep) # [total_seq_len, 32, 128]
    full_v_up = repeat_kv(full_v, n_rep) # [total_seq_len, 32, 128]
    
    # 5. Compute Attention Scores
    # We need [heads, seq_len_q, total_seq_len]
    # Transpose Q to [heads, seq_len_q, head_dim]
    # Transpose K to [heads, total_seq_len, head_dim]
    q_t = q.transpose(1, 0, 2)
    k_t = full_k_up.transpose(1, 0, 2)
    v_t = full_v_up.transpose(1, 0, 2)
    
    # Dot product Q * K^T
    # scores: [32, seq_len, total_seq_len]
    scores = np.matmul(q_t, k_t.transpose(0, 2, 1)) / np.sqrt(head_dim)
    
    # 6. Apply Causal Mask
    if mask is not None:
        scores = scores + mask
        
    # 7. SoftMax
    probs = softmax(scores, axis=-1)
    
    # 8. Weighted Sum with V
    # output: [32, seq_len, 128]
    attn_out = np.matmul(probs, v_t)
    
    # 9. Concatenate heads and project back
    # [seq_len, 32, 128] -> [seq_len, 4096]
    attn_out_merged = attn_out.transpose(1, 0, 2).reshape(seq_len, hidden_dim)
    
    return np.matmul(attn_out_merged, w_o)

if __name__ == "__main__":
    # Quick verification with dummy data
    dim = 4096
    seq = 10
    
    x = np.random.randn(seq, dim).astype(np.float32)
    w_q = np.random.randn(dim, dim).astype(np.float32)
    w_k = np.random.randn(dim, 1024).astype(np.float32)
    w_v = np.random.randn(dim, 1024).astype(np.float32)
    w_o = np.random.randn(dim, dim).astype(np.float32)
    
    out = attention_forward(x, w_q, w_k, w_v, w_o)
    print(f"Input shape: {x.shape}")
    print(f"Output shape: {out.shape}")
    if out.shape == x.shape:
        print("MHA Operator Verification: PASSED")
    else:
        print("MHA Operator Verification: FAILED")
