import numpy as np
import sys

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

def run_verification():
    # Example verification for Llama 3 8B head dim
    d_k = 128
    scale = np.sqrt(d_k)
    
    print(f"--- SoftMax Operator Verification ---")
    print(f"Head Dim (d_k): {d_k}")
    print(f"Scale Factor: {scale:.4f}")
    
    # Dummy Q and K for 10 tokens
    q = np.random.randn(10, d_k).astype(np.float32)
    k = np.random.randn(10, d_k).astype(np.float32)
    
    # QK^T / sqrt(d_k)
    scores = np.matmul(q, k.T) / scale
    
    # Softmax
    temperature = 0.7
    probs = softmax(scores, temperature=temperature)
    
    print(f"Scores Input Shape: {scores.shape}")
    print(f"Temperature: {temperature}")
    
    # Check probability distribution properties
    sums = np.sum(probs, axis=-1)
    is_valid = np.allclose(sums, 1.0, atol=1e-5)
    
    print(f"Probabilities Row Sums (should be 1.0): {sums}")
    print(f"Validation Result: {'PASSED' if is_valid else 'FAILED'}")
    
    # Check max preservation
    input_max_idx = np.argmax(scores, axis=-1)
    output_max_idx = np.argmax(probs, axis=-1)
    matches = np.array_equal(input_max_idx, output_max_idx)
    print(f"Max Value Preservation: {'PASSED' if matches else 'FAILED'}")

if __name__ == "__main__":
    run_verification()
